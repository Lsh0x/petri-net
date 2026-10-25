use serde::{Deserialize, Serialize};

pub type PlaceId = String;

/// A place with application-defined metadata.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct Place<P = ()> {
    pub id: PlaceId,
    pub data: P,
}

impl<P> Place<P> {
    pub fn new(id: impl Into<PlaceId>, data: P) -> Self {
        Self {
            id: id.into(),
            data,
        }
    }
}
