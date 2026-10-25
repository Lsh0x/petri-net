use serde::{Deserialize, Serialize};
use std::collections::HashMap;

use crate::place::PlaceId;
use crate::token::Token;

/// Tokens grouped by place.
///
/// Token IDs are caller-managed; insertion does not enforce uniqueness.
/// Deserialized markings should come from a trusted, consistent producer.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Marking<D = ()> {
    tokens: HashMap<PlaceId, Vec<Token<D>>>,
}

impl<D> Default for Marking<D> {
    fn default() -> Self {
        Self::new()
    }
}

impl<D> Marking<D> {
    pub fn new() -> Self {
        Self { tokens: HashMap::new() }
    }

    pub fn tokens_in(&self, place: &PlaceId) -> &[Token<D>] {
        self.tokens.get(place).map(Vec::as_slice).unwrap_or(&[])
    }

    /// Inserts a token, setting its place ID to the destination bucket.
    pub fn put_token(&mut self, place: PlaceId, mut token: Token<D>) {
        token.place_id = place.clone();
        self.tokens.entry(place).or_default().push(token);
    }

    /// Removes the first matching token. Missing tokens leave the marking unchanged.
    pub fn remove_token(&mut self, place: &PlaceId, token_id: u64) -> Option<Token<D>> {
        let tokens = self.tokens.get_mut(place)?;
        let pos = tokens.iter().position(|token| token.id == token_id)?;
        let token = tokens.remove(pos);
        if tokens.is_empty() {
            self.tokens.remove(place);
        }
        Some(token)
    }
}

impl<D: Clone> Marking<D> {
    /// Moves a token and returns a snapshot of it at its destination.
    pub fn move_token(&mut self, from: &PlaceId, to: PlaceId, token_id: u64) -> Option<Token<D>> {
        let mut token = self.remove_token(from, token_id)?;
        token.place_id = to;
        self.put_token(token.place_id.clone(), token.clone());
        Some(token)
    }
}
