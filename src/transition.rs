use serde::{Deserialize, Serialize};

use crate::place::PlaceId;

pub type TransId = String;

/// A directed transition with one source and one destination place.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct Transition<T = ()> {
    pub id: TransId,
    pub from: PlaceId,
    pub to: PlaceId,
    pub data: T,
}

impl<T> Transition<T> {
    pub fn new(
        id: impl Into<TransId>,
        from: impl Into<PlaceId>,
        to: impl Into<PlaceId>,
        data: T,
    ) -> Self {
        Self {
            id: id.into(),
            from: from.into(),
            to: to.into(),
            data,
        }
    }
}
