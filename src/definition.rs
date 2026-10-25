use serde::{Deserialize, Serialize};

use crate::{Marking, Place, Transition};

/// A generic net definition.
///
/// Construction does not validate duplicate IDs or dangling endpoints.
/// Applications are responsible for supplying a consistent definition.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Definition<P = (), T = ()> {
    pub places: Vec<Place<P>>,
    pub transitions: Vec<Transition<T>>,
}

impl<P, T> Definition<P, T> {
    pub fn new(places: Vec<Place<P>>, transitions: Vec<Transition<T>>) -> Self {
        Self {
            places,
            transitions,
        }
    }

    pub fn place(&self, id: &str) -> Option<&Place<P>> {
        self.places.iter().find(|place| place.id == id)
    }

    pub fn transition(&self, id: &str) -> Option<&Transition<T>> {
        self.transitions
            .iter()
            .find(|transition| transition.id == id)
    }

    /// Lists transitions whose source contains at least one token.
    ///
    /// This pure query does not evaluate domain guards or fire transitions.
    pub fn enabled_transitions<D>(&self, marking: &Marking<D>) -> Vec<&Transition<T>> {
        self.transitions
            .iter()
            .filter(|transition| !marking.tokens_in(&transition.from).is_empty())
            .collect()
    }
}
