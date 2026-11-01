//! Les réglages du clip choisi : le panneau flottant de la scène — §23.9 de
//! `docs/ARCHITECTURE.md`.
//!
//! Le même modèle que le panneau d'un trig (§23.7) : flottant, absent quand rien
//! n'est choisi, et un potentiomètre pour chaque réglage. Un `Param` modulé porte
//! une couronne : ce que sa modulation parcourt, et où elle en est.
//!
//! **Une différence, voulue : il ne couvre pas l'image.** Un spectre se lit encore
//! sous un panneau ; une image de sortie doit se voir entière. Il flotte donc à
//! côté d'elle, et ne se pose dessus que si la scène n'a plus de place.

use crate::color_panel::{color_row, swatch_button, Swatch};
use crate::widgets::{floating_panel, knob, knob_with_menu, Knob, Modulation, KNOB_STEP};
use crate::{theme, FrameView, UiState};
use egui::{Button, Rect, RichText, Ui};
use frogenx_composition::{
    Clip, ClipId, ClipParam, ClipSource, Command, Layer, LayerId, PlayMode, TransitionKind,
    MAX_CLIP_SPEED,
};
use frogenx_core::FeatureKind;
use std::ops::Range;

/// La largeur qu'on garde à côté de l'image pour le panneau.
pub const RESERVED_WIDTH: f32 = 440.0;
/// La scène la plus étroite où image et panneau tiennent côte à côte.
pub const MIN_SIDE_BY_SIDE: f32 = 800.0;
/// L'écart entre deux cartes d'une même rangée.
const CARD_GAP: f32 = 14.0;

/// Combien de potentiomètres tiennent sur une rangée de `width`. Au moins un : un
/// panneau étroit défile, il ne perd pas ses réglages.
pub fn knobs_per_row(width: f32) -> usize {
    ((width / KNOB_STEP) as usize).max(1)
}

/// Range des cartes de `counts` potentiomètres en rangées de `per_row` au plus :
/// rend les cartes de chaque rangée. Une carte plus large qu'une rangée en occupe
/// une entière, et replie ses propres potentiomètres.
///
/// **Calculé, pas confié à `horizontal_wrapped` :** egui ne replie que ce dont il
/// connaît la taille d'avance. Une carte y reste sur sa ligne, coupée au bord du
/// panneau, alors qu'il reste de la place dessous.
pub fn wrap_cards(counts: &[usize], per_row: usize) -> Vec<Range<usize>> {
    let mut rangees = Vec::new();
    let (mut debut, mut places) = (0, 0);
    for (i, n) in counts.iter().enumerate() {
        if i > debut && places + n > per_row {
            rangees.push(debut..i);
            (debut, places) = (i, 0);
        }
        places += n;
    }
    if debut < counts.len() {
        rangees.push(debut..counts.len());
    }
    rangees
}

/// La durée proposée quand on cale une vidéo sur le tempo : une mesure de quatre.
const VIDEO_DEFAULT_BEATS: f32 = 16.0;

/// Une partie du clip : son titre, sa pastille si c'est une couleur, et ses
/// potentiomètres par rangées. Rend `true` si la pastille a été cliquée.
fn card<T>(
    ui: &mut Ui,
    titre: &str,
    pastille: Option<(&Swatch, bool)>,
    items: &[T],
    per_row: usize,
    entete: impl FnOnce(&mut Ui),
    mut add: impl FnMut(&mut Ui, &T),
) -> bool {
    let mut clic = false;
    ui.vertical(|ui| {
        ui.horizontal(|ui| {
            ui.label(RichText::new(titre).size(10.5).color(theme::TEXT_DIM));
            if let Some((couleur, ouvert)) = pastille {
                clic = swatch_button(ui, titre, couleur, ouvert);
            }
            entete(ui);
        });
        for rangee in items.chunks(per_row) {
            ui.horizontal(|ui| {
                for item in rangee {
                    add(ui, item);
                }
            });
        }
    });
    clic
}

/// Les flèches d'un effet : la chaîne se lit de gauche à droite. Rend sa nouvelle
/// place, si l'une est cliquée.
fn effect_arrows(ui: &mut Ui, index: usize, chaine: usize) -> Option<usize> {
    let mut place = None;
    ui.spacing_mut().item_spacing.x = 2.0;
    let fleche = |texte| Button::new(RichText::new(texte).size(10.5)).small();
    if ui.add_enabled(index > 0, fleche("<")).clicked() {
        place = Some(index - 1);
    }
    if ui.add_enabled(index + 1 < chaine, fleche(">")).clicked() {
        place = Some(index + 1);
    }
    place
}

/// Ce qu'on peut brancher sur un paramètre depuis son menu.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Source {
    Audio(FeatureKind),
    Trig(u32),
    Nothing,
}

/// Le choix d'une source, au clic droit d'un potentiomètre (§7.1, §3bis.6). Rend
/// le nouveau choix, s'il y en a un.
fn source_menu(
    ui: &mut Ui,
    actuelle: Option<FeatureKind>,
    trig: Option<u32>,
    trigs: &[(String, u32)],
) -> Option<Source> {
    let mut choix = None;
    ui.label(
        RichText::new("Source audio")
            .color(theme::TEXT_DIM)
            .size(11.0),
    );
    for feature in FeatureKind::MENU {
        if ui
            .selectable_label(actuelle == Some(feature), feature.label())
            .clicked()
        {
            choix = Some(Source::Audio(feature));
            ui.close();
        }
    }
    if !trigs.is_empty() {
        ui.separator();
        ui.label(RichText::new("Trig").color(theme::TEXT_DIM).size(11.0));
    }
    for (nom, canal) in trigs {
        if ui.selectable_label(trig == Some(*canal), nom).clicked() {
            choix = Some(Source::Trig(*canal));
            ui.close();
        }
    }
    ui.separator();
    let debrancher = egui::Button::new("Sans source audio");
    let branche = actuelle.is_some() || trig.is_some();
    if ui.add_enabled(branche, debrancher).clicked() {
        choix = Some(Source::Nothing);
        ui.close();
    }
    choix
}

/// Le réglage d'un `Param` du clip. Il émet une commande, n'écrit rien (§23).
fn param_knob(
    ui: &mut Ui,
    clip: ClipId,
    graph: &Clip,
    param: &ClipParam,
    trigs: &[(String, u32)],
    out: &mut Vec<Command>,
) {
    ui.spacing_mut().item_spacing.x = 8.0;
    let desc = param.desc;
    let nom = desc.name.to_uppercase();
    let modulation = desc.value.depth().map(|depth| Modulation {
        depth,
        live: desc.value.get(graph.graph()),
    });
    // Modulé, le potentiomètre règle la base : la modulation continue.
    let couleur = theme::ACCENT;
    let reglage =
        Knob::new(&nom, desc.value.base(), desc.min, desc.max, couleur).modulated(modulation);
    let mut demande = None;
    let mut menu = |ui: &mut Ui| demande = source_menu(ui, param.audio, param.trig, trigs);
    let change = knob_with_menu(ui, &reglage, Some(&mut menu));
    let slot = param.slot;
    match demande {
        Some(Source::Audio(feature)) => out.push(Command::SetAudioModulation {
            clip,
            slot,
            feature: Some(feature),
        }),
        Some(Source::Trig(channel)) => out.push(Command::SetTrigModulation {
            clip,
            slot,
            channel,
        }),
        Some(Source::Nothing) => out.push(Command::SetAudioModulation {
            clip,
            slot,
            feature: None,
        }),
        None => {}
    }
    if let Some(value) = change.value {
        out.push(Command::SetParam {
            clip,
            slot: param.slot,
            value,
        });
    }
    if let Some(value) = change.depth {
        out.push(Command::SetDepth {
            clip,
            slot: param.slot,
            value,
        });
    }
}

/// Comment ce clip entre quand un autre jouait : un choix, et sa durée.
fn transition_row(ui: &mut Ui, id: ClipId, clip: &Clip, commands: &mut Vec<Command>) {
    let nom = ui.label(
        RichText::new("Transition")
            .size(11.0)
            .color(theme::TEXT_DIM),
    );
    let actuelle = clip.transition_kind();
    let beats = clip.transition_beats();
    egui::ComboBox::from_id_salt(("transition", id))
        .selected_text(actuelle.label())
        .width(150.0)
        .show_ui(ui, |ui| {
            for kind in TransitionKind::ALL {
                if ui
                    .selectable_label(kind == actuelle, kind.label())
                    .clicked()
                {
                    commands.push(Command::SetClipTransition {
                        clip: id,
                        kind,
                        beats,
                    });
                }
            }
        })
        .response
        .labelled_by(nom.id);
    if actuelle != TransitionKind::Cut {
        let mut duree = beats;
        let reglage = egui::DragValue::new(&mut duree)
            .range(0.25..=64.0)
            .speed(0.05)
            .suffix(" temps")
            .max_decimals(2);
        if ui.add(reglage).labelled_by(nom.id).changed() {
            commands.push(Command::SetClipTransition {
                clip: id,
                kind: actuelle,
                beats: duree,
            });
        }
    }
}

/// Ce que fait le clip au bout de sa durée, et cette durée.
fn playback_row(ui: &mut Ui, id: ClipId, clip: &Clip, commands: &mut Vec<Command>) {
    let Some((actuel, beats)) = clip.playback() else {
        return;
    };
    let nom = ui.label(RichText::new("Lecture").size(11.0).color(theme::TEXT_DIM));
    egui::ComboBox::from_id_salt(("lecture", id))
        .selected_text(actuel.label())
        .width(150.0)
        .show_ui(ui, |ui| {
            for mode in PlayMode::ALL {
                if ui.selectable_label(mode == actuel, mode.label()).clicked() {
                    commands.push(Command::SetClipPlayback {
                        clip: id,
                        mode,
                        beats,
                    });
                }
            }
        })
        .response
        .labelled_by(nom.id);
    let mut duree = beats;
    let reglage = egui::DragValue::new(&mut duree)
        .range(0.25..=256.0)
        .speed(0.05)
        .suffix(" temps")
        .max_decimals(2);
    if ui.add(reglage).labelled_by(nom.id).changed() {
        commands.push(Command::SetClipPlayback {
            clip: id,
            mode: actuel,
            beats: duree,
        });
    }
}

/// La lecture d'une vidéo : son mode, et le nombre de temps que dure sa boucle.
/// `0` temps : sa vitesse réelle (§9.3).
fn video_row(ui: &mut Ui, id: ClipId, clip: &Clip, commands: &mut Vec<Command>) {
    let Some((_, actuel, beats)) = clip.video_playback() else {
        return;
    };
    let nom = ui.label(RichText::new("Lecture").size(11.0).color(theme::TEXT_DIM));
    egui::ComboBox::from_id_salt(("lecture vidéo", id))
        .selected_text(actuel.label())
        .width(120.0)
        .show_ui(ui, |ui| {
            for mode in PlayMode::ALL {
                if ui.selectable_label(mode == actuel, mode.label()).clicked() {
                    commands.push(Command::SetVideoPlayback {
                        clip: id,
                        mode,
                        beats,
                    });
                }
            }
        })
        .response
        .labelled_by(nom.id);
    let calee = beats > 0.0;
    if ui.selectable_label(calee, "Caler sur").clicked() {
        commands.push(Command::SetVideoPlayback {
            clip: id,
            mode: actuel,
            beats: if calee { 0.0 } else { VIDEO_DEFAULT_BEATS },
        });
    }
    if calee {
        let mut duree = beats;
        let reglage = egui::DragValue::new(&mut duree)
            .range(0.25..=256.0)
            .speed(0.05)
            .suffix(" temps")
            .max_decimals(2);
        if ui.add(reglage).changed() {
            commands.push(Command::SetVideoPlayback {
                clip: id,
                mode: actuel,
                beats: duree,
            });
        }
    }
}

/// Les réglages propres au clip et à sa piste.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ClipKnob {
    Speed,
    Opacity,
}

const CLIP_KNOBS: [ClipKnob; 2] = [ClipKnob::Speed, ClipKnob::Opacity];

fn clip_knob(
    ui: &mut Ui,
    reglage: ClipKnob,
    id: ClipId,
    layer: LayerId,
    clip: &Clip,
    piste: &Layer,
    commands: &mut Vec<Command>,
) {
    ui.spacing_mut().item_spacing.x = 8.0;
    match reglage {
        ClipKnob::Speed => {
            let vitesse = Knob::new(
                "VITESSE",
                clip.speed.base(),
                0.0,
                MAX_CLIP_SPEED,
                theme::ACCENT,
            );
            if let Some(value) = knob(ui, &vitesse).value {
                commands.push(Command::SetClipSpeed { clip: id, value });
            }
        }
        ClipKnob::Opacity => {
            let mut opacity = piste.blend.opacity;
            let reglage = Knob::new("OPACITÉ", opacity.base(), 0.0, 1.0, theme::ACCENT);
            if let Some(value) = knob(ui, &reglage).value {
                opacity.set_base(value);
                commands.push(Command::SetLayerOpacity { layer, opacity });
            }
        }
    }
}

/// Le panneau du clip choisi, dans `cadre`.
pub fn floating(
    ui: &mut Ui,
    cadre: Rect,
    state: &mut UiState,
    view: &FrameView<'_>,
    commands: &mut Vec<Command>,
    (layer, id): (LayerId, ClipId),
) {
    let c = view.composition;
    let (Some(piste), Some(clip)) = (c.layer(layer), c.clip(id)) else {
        return;
    };
    floating_panel(ui, cadre, "fond_clip", |ui| {
        ui.vertical(|ui| {
            ui.horizontal(|ui| {
                ui.label(RichText::new(&clip.name).color(theme::ACCENT).strong());
                ui.label(RichText::new(&piste.name).color(theme::TEXT_DIM).size(11.0));
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui.small_button("Fermer").clicked() {
                        state.selected = None;
                    }
                });
            });
            if let Some(erreur) = view.video_error {
                ui.label(RichText::new(erreur).size(11.0).color(theme::ACCENT));
            }
            ui.horizontal(|ui| playback_row(ui, id, clip, commands));
            ui.horizontal(|ui| video_row(ui, id, clip, commands));
            ui.horizontal(|ui| transition_row(ui, id, clip, commands));
            let groupes = clip.params();
            let chaine = match clip.source {
                ClipSource::Shape { .. } => clip.effects.len(),
                ClipSource::Feedback { .. } | ClipSource::Video { .. } => 0,
            };
            let trigs: Vec<(String, u32)> = c
                .trigs()
                .iter()
                .filter_map(|t| c.trig(*t))
                .map(|t| (t.name.clone(), t.channel()))
                .collect();
            let couleurs: Vec<Option<Swatch>> =
                groupes.iter().map(|g| Swatch::of(clip, g)).collect();
            let mut places = vec![CLIP_KNOBS.len()];
            places.extend(groupes.iter().map(|g| g.params.len()));
            egui::ScrollArea::vertical().show(ui, |ui| {
                ui.set_width(ui.available_width());
                // Un écart de carte retiré par carte : large, mais jamais coupé.
                let par_rangee = knobs_per_row(ui.available_width() - CARD_GAP);
                let ouverte = state.open_color.filter(|(c, _)| *c == id).map(|o| o.1);
                let selecteur = couleurs.iter().zip(&groupes).find_map(|(c, g)| {
                    let couleur = c.as_ref().filter(|c| Some(c.key) == ouverte)?;
                    Some((couleur, &g.label))
                });
                if let Some((couleur, titre)) = selecteur {
                    color_row(ui, titre, id, couleur, commands);
                }
                for rangee in wrap_cards(&places, par_rangee) {
                    ui.horizontal_top(|ui| {
                        ui.spacing_mut().item_spacing.x = CARD_GAP;
                        for i in rangee {
                            if i == 0 {
                                let ajout = |ui: &mut Ui, reglage: &ClipKnob| {
                                    clip_knob(ui, *reglage, id, layer, clip, piste, commands);
                                };
                                card(ui, "Clip", None, &CLIP_KNOBS, par_rangee, |_| {}, ajout);
                            } else {
                                let groupe = &groupes[i - 1];
                                let couleur = couleurs[i - 1].as_ref();
                                let cle = couleur.map(|c| c.key);
                                let ouvert = cle.is_some() && cle == ouverte;
                                // Le groupe 0 est la forme : les effets viennent après.
                                let effet = (i >= 2).then(|| i - 2).filter(|e| *e < chaine);
                                let mut deplace = None;
                                let clic = card(
                                    ui,
                                    &groupe.label,
                                    couleur.map(|c| (c, ouvert)),
                                    &groupe.params,
                                    par_rangee,
                                    |ui| {
                                        if let Some(index) = effet {
                                            deplace = effect_arrows(ui, index, chaine)
                                                .map(|to| (index, to));
                                        }
                                    },
                                    |ui, param| {
                                        param_knob(ui, id, clip, param, &trigs, commands);
                                    },
                                );
                                if let Some((index, to)) = deplace {
                                    commands.push(Command::MoveEffect {
                                        clip: id,
                                        index,
                                        to,
                                    });
                                }
                                if clic {
                                    state.open_color = cle.filter(|_| !ouvert).map(|cle| (id, cle));
                                }
                            }
                        }
                    });
                }
            });
        });
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn les_cartes_remplissent_chaque_rangee_puis_passent_a_la_suivante() {
        assert_eq!(wrap_cards(&[2, 3, 3, 2], 6), vec![0..2, 2..4]);
    }

    #[test]
    fn une_carte_trop_large_occupe_sa_propre_rangee() {
        assert_eq!(wrap_cards(&[2, 9, 1], 6), vec![0..1, 1..2, 2..3]);
    }

    #[test]
    fn la_premiere_carte_ne_laisse_jamais_de_rangee_vide() {
        assert_eq!(wrap_cards(&[9], 6), vec![0..1]);
        assert_eq!(wrap_cards(&[], 6), Vec::<Range<usize>>::new());
    }

    #[test]
    fn toutes_les_cartes_sont_rangees_une_seule_fois() {
        let cartes = [2, 3, 3, 3, 2, 4, 8, 1];
        for par_rangee in 1..12 {
            let mut vues = Vec::new();
            for rangee in wrap_cards(&cartes, par_rangee) {
                vues.extend(rangee);
            }
            assert_eq!(vues, (0..cartes.len()).collect::<Vec<_>>());
        }
    }

    #[test]
    fn un_panneau_etroit_garde_au_moins_un_potentiometre_par_rangee() {
        assert_eq!(knobs_per_row(0.0), 1);
        assert_eq!(knobs_per_row(KNOB_STEP * 6.5), 6);
    }
}
