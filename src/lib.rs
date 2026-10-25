#![doc = include_str!("../README.md")]

pub mod definition;
pub mod marking;
pub mod place;
pub mod token;
pub mod transition;

pub use definition::Definition;
pub use marking::Marking;
pub use place::{Place, PlaceId};
pub use token::Token;
pub use transition::{TransId, Transition};

/// Authenticated numeric workflow engine, separate from the legacy data model.
pub mod engine;

pub use engine::Petri;
