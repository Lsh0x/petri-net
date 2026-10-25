//! Generic data model for state-machine Petri nets.
//!
//! Transitions have one source and one destination. Domain-specific rules,
//! scheduling and transition execution belong in the consuming application.
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
