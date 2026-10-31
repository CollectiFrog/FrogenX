//! La grille de départ : ce qu'on trouve au lancement, sans sauvegarde (§23.6).

use crate::{find, preset};
use frogenx_composition::{Clip, ColumnId, Composition, LayerId};
use frogenx_core::Param;
use frogenx_osc::{Lfo, Waveform};
use frogenx_shape::{BlendMode, BlendParams};

/// Le nombre de colonnes de la grille de départ.
pub const STARTER_COLUMNS: usize = 8;

/// Fond, forme et trait de chaque colonne. Un fond absent laisse jouer le
/// précédent : c'est une case vide (§3bis.1).
const COLUMNS: [(Option<&str>, &str, &str); STARTER_COLUMNS] = [
    (Some("Nuit"), "Blob", "Étoile"),
    (None, "Rosace", "Lissajous"),
    (None, "Polygone", "Anneau"),
    (None, "Fleur", "Superformule"),
    (Some("Aurore"), "Mandala", "Arcs"),
    (Some("Marée"), "Cristal", "Spirographe"),
    (Some("Braise"), "Engrenage", "Anneau"),
    (None, "Kaléidoscope", "Superformule"),
];

/// Un clip neuf du catalogue. Le test de la grille prouve que chaque nom
/// existe.
fn clip(nom: &str) -> Clip {
    find(nom)
        .and_then(preset)
        .expect("la grille de départ ne cite que le catalogue")
        .build()
}

fn poser(c: &mut Composition, piste: LayerId, colonne: ColumnId, nom: &str) {
    c.place_clip(piste, colonne, clip(nom))
        .expect("la piste et la colonne viennent d'être créées");
}

/// Trois pistes et huit colonnes. Le fond joue déjà ; aucune colonne n'est
/// déclenchée.
pub fn starter() -> Composition {
    let mut c = Composition::new();
    let deck = c.add_deck("Principal");
    let groupe = c.add_layer_group(deck).expect("le deck vient d'être créé");
    let piste = |c: &mut Composition| c.add_layer(groupe).expect("le groupe vient d'être créé");
    let (arriere, formes, traits) = (piste(&mut c), piste(&mut c), piste(&mut c));
    for (id, nom) in [(arriere, "Fond"), (formes, "Formes"), (traits, "Traits")] {
        if let Some(l) = c.layer_mut(id) {
            l.name = nom.to_string();
        }
    }

    // L'opacité des traits respire : elle appartient à la piste, pas au clip,
    // et continue donc de respirer d'un clip à l'autre (§3bis.1).
    let respiration = c
        .graph_mut()
        .add(Box::new(Lfo::new(Waveform::Sine, 0.25).unipolar()));
    if let Some(l) = c.layer_mut(traits) {
        l.blend = BlendParams {
            opacity: Param::modulated(0.45, 0.55, respiration),
            mode: BlendMode::Add,
        };
    }

    for (fond, forme, trait_) in COLUMNS {
        let colonne = c.add_column();
        if let Some(fond) = fond {
            poser(&mut c, arriere, colonne, fond);
        }
        poser(&mut c, formes, colonne, forme);
        poser(&mut c, traits, colonne, trait_);
    }

    let premier = c.layer(arriere).and_then(|l| l.cell(0));
    if let Some(fond) = premier {
        c.trigger(arriere, fond);
    }
    c
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pistes(c: &Composition) -> Vec<LayerId> {
        let mut pistes = Vec::new();
        c.layers_in_order(&mut pistes);
        pistes
    }

    #[test]
    fn la_grille_a_trois_pistes_et_huit_colonnes() {
        let c = starter();
        assert_eq!(pistes(&c).len(), 3);
        assert_eq!(c.columns().len(), STARTER_COLUMNS);
    }

    #[test]
    fn le_fond_joue_et_aucune_colonne_n_est_declenchee() {
        let c = starter();
        let [fond, formes, traits] = pistes(&c)[..] else {
            panic!("trois pistes attendues");
        };
        let actif = c.layer(fond).unwrap().active_clip().unwrap();
        assert_eq!(c.clip(actif).unwrap().name, "Nuit");
        assert_eq!(c.layer(formes).unwrap().active_clip(), None);
        assert_eq!(c.layer(traits).unwrap().active_clip(), None);
    }

    #[test]
    fn chaque_colonne_remplit_les_pistes_de_formes_et_de_traits() {
        let c = starter();
        let ordre = pistes(&c);
        for colonne in 0..STARTER_COLUMNS {
            for piste in &ordre[1..] {
                assert!(
                    c.layer(*piste).unwrap().cell(colonne).is_some(),
                    "colonne {colonne}"
                );
            }
        }
    }

    #[test]
    fn une_colonne_sans_fond_garde_le_fond_qui_joue() {
        let mut c = starter();
        let ordre = pistes(&c);
        let avant = c.layer(ordre[0]).unwrap().active_clip();
        let colonne = c.columns()[1];
        assert!(c.trigger_column(colonne, &ordre));
        assert_eq!(c.layer(ordre[0]).unwrap().active_clip(), avant);
    }

    #[test]
    fn une_colonne_avec_fond_le_remplace() {
        let mut c = starter();
        let ordre = pistes(&c);
        let colonne = c.columns()[4];
        assert!(c.trigger_column(colonne, &ordre));
        let actif = c.layer(ordre[0]).unwrap().active_clip().unwrap();
        assert_eq!(c.clip(actif).unwrap().name, "Aurore");
    }
}
