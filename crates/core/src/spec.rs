//! Décrire un nœud sans connaître le format de fichier — §19.1 de
//! `docs/ARCHITECTURE.md`.
//!
//! `core` ne dépend de rien : une [`Spec`] est une simple valeur, que `patch`
//! traduit en JSON. Un nœud se reconstruit depuis sa `Spec` par le `from_spec`
//! de son type.

use crate::param::Param;
use std::fmt;

/// La valeur d'un champ.
#[derive(Debug, Clone, PartialEq)]
pub enum SpecValue {
    Param(Param),
    Params(Vec<Param>),
    Number(f32),
    Int(u32),
    Bool(bool),
    /// Un choix parmi des mots fixes : une forme d'onde, un mode.
    Word(String),
    Specs(Vec<Spec>),
}

impl From<Param> for SpecValue {
    fn from(v: Param) -> Self {
        Self::Param(v)
    }
}

impl From<Vec<Param>> for SpecValue {
    fn from(v: Vec<Param>) -> Self {
        Self::Params(v)
    }
}

impl From<f32> for SpecValue {
    fn from(v: f32) -> Self {
        Self::Number(v)
    }
}

impl From<u32> for SpecValue {
    fn from(v: u32) -> Self {
        Self::Int(v)
    }
}

impl From<bool> for SpecValue {
    fn from(v: bool) -> Self {
        Self::Bool(v)
    }
}

impl From<&str> for SpecValue {
    fn from(v: &str) -> Self {
        Self::Word(v.to_string())
    }
}

impl From<Vec<Spec>> for SpecValue {
    fn from(v: Vec<Spec>) -> Self {
        Self::Specs(v)
    }
}

/// Une `Spec` refusée : régime d'édition (§18.1).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SpecError {
    /// Aucun type ne porte ce nom.
    UnknownKind(String),
    /// Le champ manque, ou n'a pas le type attendu.
    Field { kind: String, field: String },
}

impl fmt::Display for SpecError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::UnknownKind(kind) => write!(f, "type inconnu : « {kind} »"),
            Self::Field { kind, field } => {
                write!(f, "« {kind} » : champ « {field} » absent ou invalide")
            }
        }
    }
}

impl std::error::Error for SpecError {}

/// Un nœud, une forme ou un effet, décrit par son type et ses champs.
#[derive(Debug, Clone, PartialEq)]
pub struct Spec {
    pub kind: String,
    pub fields: Vec<(String, SpecValue)>,
}

impl Spec {
    pub fn new(kind: impl Into<String>) -> Self {
        Self {
            kind: kind.into(),
            fields: Vec::new(),
        }
    }

    pub fn with(mut self, name: &str, value: impl Into<SpecValue>) -> Self {
        self.fields.push((name.to_string(), value.into()));
        self
    }

    pub fn get(&self, name: &str) -> Option<&SpecValue> {
        self.fields.iter().find(|(n, _)| n == name).map(|(_, v)| v)
    }

    /// Ses `Param`, listes dépliées : ce que l'éditeur appelle ses entrées.
    pub fn params_flat(&self) -> Vec<Param> {
        let mut params = Vec::new();
        for (_, value) in &self.fields {
            match value {
                SpecValue::Param(p) => params.push(*p),
                SpecValue::Params(list) => params.extend_from_slice(list),
                _ => {}
            }
        }
        params
    }

    /// Le nom de chaque entrée, dans l'ordre de [`Self::params_flat`].
    pub fn inputs(&self) -> Vec<String> {
        let mut noms = Vec::new();
        for (name, value) in &self.fields {
            match value {
                SpecValue::Param(_) => noms.push(name.clone()),
                SpecValue::Params(list) => {
                    noms.extend((0..list.len()).map(|i| format!("{name} {}", i + 1)));
                }
                _ => {}
            }
        }
        noms
    }

    /// Écrit la `index`-ième entrée. Rend `false` si elle n'existe pas.
    pub fn set_flat(&mut self, index: usize, param: Param) -> bool {
        let mut reste = index;
        for (_, value) in &mut self.fields {
            match value {
                SpecValue::Param(p) => {
                    if reste == 0 {
                        *p = param;
                        return true;
                    }
                    reste -= 1;
                }
                SpecValue::Params(list) => {
                    if let Some(p) = list.get_mut(reste) {
                        *p = param;
                        return true;
                    }
                    reste -= list.len();
                }
                _ => {}
            }
        }
        false
    }

    /// L'erreur d'un champ absent ou mal typé.
    pub fn field_error(&self, name: &str) -> SpecError {
        SpecError::Field {
            kind: self.kind.clone(),
            field: name.to_string(),
        }
    }

    pub fn param(&self, name: &str) -> Result<Param, SpecError> {
        match self.get(name) {
            Some(SpecValue::Param(p)) => Ok(*p),
            _ => Err(self.field_error(name)),
        }
    }

    pub fn params(&self, name: &str) -> Result<&[Param], SpecError> {
        match self.get(name) {
            Some(SpecValue::Params(p)) => Ok(p),
            _ => Err(self.field_error(name)),
        }
    }

    /// JSON ne distingue pas `2` de `2.0` : un entier fait un nombre valable.
    pub fn number(&self, name: &str) -> Result<f32, SpecError> {
        match self.get(name) {
            Some(SpecValue::Number(v)) => Ok(*v),
            Some(SpecValue::Int(v)) => Ok(*v as f32),
            _ => Err(self.field_error(name)),
        }
    }

    pub fn int(&self, name: &str) -> Result<u32, SpecError> {
        match self.get(name) {
            Some(SpecValue::Int(v)) => Ok(*v),
            _ => Err(self.field_error(name)),
        }
    }

    pub fn bool(&self, name: &str) -> Result<bool, SpecError> {
        match self.get(name) {
            Some(SpecValue::Bool(v)) => Ok(*v),
            _ => Err(self.field_error(name)),
        }
    }

    pub fn word(&self, name: &str) -> Result<&str, SpecError> {
        match self.get(name) {
            Some(SpecValue::Word(v)) => Ok(v),
            _ => Err(self.field_error(name)),
        }
    }

    /// Un tableau vide se lit indifféremment comme une liste de `Param` ou de
    /// `Spec` : rien, dans un fichier, ne les distingue.
    pub fn specs(&self, name: &str) -> Result<&[Spec], SpecError> {
        match self.get(name) {
            Some(SpecValue::Specs(v)) => Ok(v),
            Some(SpecValue::Params(v)) if v.is_empty() => Ok(&[]),
            _ => Err(self.field_error(name)),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn un_champ_se_relit_avec_son_type() {
        let spec = Spec::new("Test")
            .with("p", Param::fixed(0.5))
            .with("n", 0.25)
            .with("i", 3u32)
            .with("b", true)
            .with("w", "sine");
        assert_eq!(spec.param("p"), Ok(Param::fixed(0.5)));
        assert_eq!(spec.number("n"), Ok(0.25));
        assert_eq!(spec.int("i"), Ok(3));
        assert_eq!(spec.bool("b"), Ok(true));
        assert_eq!(spec.word("w"), Ok("sine"));
    }

    #[test]
    fn un_champ_absent_ou_mal_type_nomme_le_type_et_le_champ() {
        let spec = Spec::new("Lfo").with("n", 0.25);
        let attendu = SpecError::Field {
            kind: "Lfo".into(),
            field: "freq".into(),
        };
        assert_eq!(spec.param("freq"), Err(attendu.clone()));
        assert_eq!(
            attendu.to_string(),
            "« Lfo » : champ « freq » absent ou invalide"
        );
        assert!(spec.param("n").is_err());
        assert!(spec.word("n").is_err());
    }

    #[test]
    fn un_entier_fait_un_nombre_mais_pas_l_inverse() {
        let spec = Spec::new("Test").with("i", 2u32).with("n", 2.5);
        assert_eq!(spec.number("i"), Ok(2.0));
        assert!(spec.int("n").is_err());
    }

    #[test]
    fn les_entrees_deplient_les_listes_dans_l_ordre() {
        let (a, b, c) = (Param::fixed(1.0), Param::fixed(2.0), Param::fixed(3.0));
        let mut spec = Spec::new("Test")
            .with("mot", "sine")
            .with("a", a)
            .with("liste", vec![b, c]);
        assert_eq!(spec.params_flat(), vec![a, b, c]);
        assert_eq!(spec.inputs(), ["a", "liste 1", "liste 2"]);
        assert!(spec.set_flat(2, a));
        assert_eq!(spec.params_flat(), vec![a, b, a]);
        assert!(!spec.set_flat(3, a));
    }

    #[test]
    fn un_tableau_vide_vaut_pour_les_deux_listes() {
        let spec = Spec::new("Test").with("v", Vec::<Param>::new());
        assert!(spec.params("v").unwrap().is_empty());
        assert!(spec.specs("v").unwrap().is_empty());
    }
}
