use serde::{Deserialize, Serialize};

use crate::place::PlaceId;

/// A token carrying application-defined data.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct Token<D = ()> {
    pub id: u64,
    pub place_id: PlaceId,
    pub data: D,
}

impl<D> Token<D> {
    pub fn new(id: u64, place_id: impl Into<PlaceId>, data: D) -> Self {
        Self { id, place_id: place_id.into(), data }
    }
}
