//! Identifiants arena, un type par niveau de la grille — §3bis.1 de
//! `docs/ARCHITECTURE.md`.
//!
//! Même idiome que [`frogenx_core::NodeId`] : un `u32` d'index plus un
//! compteur de génération, pour qu'un identifiant périmé ne pointe jamais
//! vers un emplacement réutilisé. Répété une fois par niveau plutôt que
//! partagé dans un identifiant générique unique : confondre un `ClipId` et
//! un `DeckId` devient une erreur de **type**, détectée à la compilation,
//! pas un bug silencieux découvert à l'exécution.

use serde::{Deserialize, Serialize};

macro_rules! arena_id {
    ($name:ident, $doc:literal) => {
        #[doc = $doc]
        #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
        pub struct $name {
            index: u32,
            gen: u32,
        }

        impl $name {
            pub(crate) fn new(index: u32, gen: u32) -> Self {
                Self { index, gen }
            }

            pub fn index(&self) -> u32 {
                self.index
            }

            pub fn generation(&self) -> u32 {
                self.gen
            }
        }
    };
}

arena_id!(DeckId, "Identifiant d'un `Deck` (§3bis.1).");
arena_id!(LayerGroupId, "Identifiant d'un `LayerGroup` (§3bis.1).");
arena_id!(LayerId, "Identifiant d'un `Layer` (§3bis.1).");
arena_id!(ColumnId, "Identifiant d'une `Column` (§3bis.1).");
arena_id!(ClipId, "Identifiant d'un `Clip` (§3bis.2).");

/// Une arena par niveau : `Vec<Option<T>>` indexé par `index()`, avec
/// compteur de génération par emplacement — même principe que
/// `frogenx_core::graph::Graph` (§3), répété ici pour chaque niveau de la
/// grille plutôt que partagé dans une structure générique.
#[derive(Debug, Default)]
pub(crate) struct Arena<T> {
    slots: Vec<Option<T>>,
    gens: Vec<u32>,
}

impl<T> Arena<T> {
    pub(crate) fn new() -> Self {
        Self {
            slots: Vec::new(),
            gens: Vec::new(),
        }
    }

    /// Insère une valeur et rend son index brut plus sa génération.
    pub(crate) fn insert(&mut self, value: T) -> (u32, u32) {
        if let Some(i) = self.slots.iter().position(|s| s.is_none()) {
            self.slots[i] = Some(value);
            (i as u32, self.gens[i])
        } else {
            self.slots.push(Some(value));
            self.gens.push(0);
            ((self.slots.len() - 1) as u32, 0)
        }
    }

    pub(crate) fn get(&self, index: u32, gen: u32) -> Option<&T> {
        if self.gens.get(index as usize).copied() != Some(gen) {
            return None;
        }
        self.slots.get(index as usize)?.as_ref()
    }

    pub(crate) fn get_mut(&mut self, index: u32, gen: u32) -> Option<&mut T> {
        if self.gens.get(index as usize).copied() != Some(gen) {
            return None;
        }
        self.slots.get_mut(index as usize)?.as_mut()
    }

    /// Retire une valeur et avance sa génération : un identifiant existant
    /// vers cet emplacement devient périmé, même si l'emplacement est réutilisé.
    pub(crate) fn remove(&mut self, index: u32, gen: u32) -> Option<T> {
        if self.gens.get(index as usize).copied() != Some(gen) {
            return None;
        }
        let taken = self.slots.get_mut(index as usize)?.take();
        if taken.is_some() {
            self.gens[index as usize] = self.gens[index as usize].wrapping_add(1);
        }
        taken
    }

    /// Les valeurs présentes, dans l'ordre des emplacements.
    pub(crate) fn iter_mut(&mut self) -> impl Iterator<Item = &mut T> {
        self.slots.iter_mut().flatten()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn un_id_perime_ne_pointe_pas_vers_le_nouveau_noeud() {
        // §16.5 (core) répété ici pour l'arena de composition.
        let mut a: Arena<i32> = Arena::new();
        let (i1, g1) = a.insert(1);
        a.remove(i1, g1);
        let (i2, g2) = a.insert(2);

        assert_eq!(i1, i2, "l'emplacement doit être réutilisé");
        assert_ne!(g1, g2, "la génération doit avancer");
        assert_eq!(a.get(i1, g1), None, "l'ancien id périmé ne résout plus");
        assert_eq!(a.get(i2, g2), Some(&2));
    }

    #[test]
    fn deck_id_et_clip_id_sont_des_types_distincts() {
        // La confusion entre niveaux est une erreur de compilation, pas un
        // test à l'exécution — ce test prouve seulement que chaque type a
        // son propre constructeur et accesseurs, pas qu'ils sont interchangeables.
        let deck = DeckId::new(0, 0);
        let clip = ClipId::new(0, 0);
        assert_eq!(deck.index(), clip.index());
        assert_eq!(deck.generation(), clip.generation());
        // `deck == clip` ne compile pas : DeckId et ClipId n'implémentent
        // pas PartialEq l'un envers l'autre. C'est la garantie recherchée.
    }
}
