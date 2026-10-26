//! Numeric, authenticated Petri execution with optimistic concurrency.
//!
//! The storage implementation is the authority for freshness. MACs authenticate
//! claims; they do not make stale snapshots current or authenticate business work.
mod storage;
pub use storage::{BoxFuture, MemoryStorage, Storage};

use hmac::{Hmac, Mac};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc as Shared;

/// Numeric place identifier.
pub type PlaceId = u32;
/// Numeric transition identifier.
pub type TransitionId = u32;
/// Application-assigned execution ID; never reuse one in the same storage namespace.
pub type RunId = u128;
/// Monotonically allocated token ID within a run.
pub type TokenId = u64;
/// SHA-256 identity of the exact serialized definition.
pub type DefinitionHash = [u8; 32];
const VERSION: u32 = 2;
const MAX_TOKENS: usize = 4096;
const MAX_BYTES: usize = 4 * 1024 * 1024;
type HmacSha256 = Hmac<Sha256>;

/// Engine and storage failures. No failed validation commits a marking change.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Error {
    InvalidDefinition,
    InvalidSnapshot,
    InvalidToken,
    InvalidInputs,
    IncompatibleScope,
    UnknownTransition,
    NotFound,
    Conflict,
    Limit,
    Encoding,
    Storage(String),
}
impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{self:?}")
    }
}
impl std::error::Error for Error {}
pub type Result<T> = std::result::Result<T, Error>;

/// A weighted arc. Duplicate places within an arc list are rejected.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Arc {
    pub place: PlaceId,
    pub weight: u32,
}
impl Arc {
    pub fn new(place: PlaceId, weight: u32) -> Self {
        Self { place, weight }
    }
}

/// Multi-input/multi-output transition. Empty outputs represent a sink.
///
/// Input tokens must share the exact fork context. A multi-token consumption
/// closes its innermost context; a multi-token production creates a fresh one.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Transition {
    pub id: TransitionId,
    pub inputs: Vec<Arc>,
    pub outputs: Vec<Arc>,
}
impl Transition {
    pub fn new(id: TransitionId, inputs: Vec<Arc>, outputs: Vec<Arc>) -> Self {
        Self {
            id,
            inputs,
            outputs,
        }
    }
}

/// Immutable definition. Construction and JSON import both validate its graph.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Definition {
    version: u32,
    places: Vec<PlaceId>,
    initial: PlaceId,
    transitions: Vec<Transition>,
    #[serde(default, skip_serializing_if = "is_false")]
    dependency_graph: bool,
}
impl Definition {
    pub fn new(
        places: Vec<PlaceId>,
        initial: PlaceId,
        transitions: Vec<Transition>,
    ) -> Result<Self> {
        let value = Self {
            version: VERSION,
            places,
            initial,
            transitions,
            dependency_graph: false,
        };
        value.validate()?;
        Ok(value)
    }
    pub(crate) fn for_dependency_graph(mut self) -> Self {
        self.dependency_graph = true;
        self
    }
    pub fn to_json(&self) -> Result<String> {
        encode_string(self)
    }
    pub fn from_json(json: &str) -> Result<Self> {
        let value: Self = decode(json)?;
        value.validate()?;
        Ok(value)
    }
    pub fn hash(&self) -> Result<DefinitionHash> {
        Ok(Sha256::digest(encode(self)?).into())
    }
    pub fn transitions(&self) -> &[Transition] {
        &self.transitions
    }
    fn transition(&self, id: TransitionId) -> Result<&Transition> {
        self.transitions
            .iter()
            .find(|t| t.id == id)
            .ok_or(Error::UnknownTransition)
    }
    fn validate(&self) -> Result<()> {
        let places: BTreeSet<_> = self.places.iter().copied().collect();
        if self.version != VERSION
            || places.is_empty()
            || places.len() != self.places.len()
            || !places.contains(&self.initial)
        {
            return Err(Error::InvalidDefinition);
        }
        let mut ids = BTreeSet::new();
        for t in &self.transitions {
            if !ids.insert(t.id) || t.inputs.is_empty() {
                return Err(Error::InvalidDefinition);
            }
            for arcs in [&t.inputs, &t.outputs] {
                let mut seen = BTreeSet::new();
                let mut count = 0usize;
                for a in arcs {
                    if a.weight == 0 || !places.contains(&a.place) || !seen.insert(a.place) {
                        return Err(Error::InvalidDefinition);
                    }
                    count = count.checked_add(a.weight as usize).ok_or(Error::Limit)?;
                }
                if count > MAX_TOKENS {
                    return Err(Error::Limit);
                }
            }
        }
        // Also bounds serialized metadata before it is accepted by the engine.
        encode(self)?;
        Ok(())
    }
}

/// Authenticated claims. All identity fields are private and covered by the MAC.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Claims {
    version: u32,
    definition: DefinitionHash,
    run: RunId,
    id: TokenId,
    place: PlaceId,
    scope: Vec<u64>,
    parents: Vec<TokenId>,
}

/// Portable token. Deserialization is not verification; use [`Petri::verify_token`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Token {
    claims: Claims,
    tag: [u8; 32],
}
impl Token {
    pub fn id(&self) -> TokenId {
        self.claims.id
    }
    pub fn run_id(&self) -> RunId {
        self.claims.run
    }
    pub fn place(&self) -> PlaceId {
        self.claims.place
    }
    pub fn scope(&self) -> &[u64] {
        &self.claims.scope
    }
    pub fn parents(&self) -> &[TokenId] {
        &self.claims.parents
    }
    pub fn to_json(&self) -> Result<String> {
        encode_string(self)
    }
    pub fn from_json(json: &str) -> Result<Self> {
        decode(json)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct State {
    version: u32,
    definition: DefinitionHash,
    run: RunId,
    revision: u64,
    next_token: u64,
    next_scope: u64,
    tokens: Vec<Token>,
}

/// Authenticated, versioned marking snapshot. Keys are never serialized.
/// A historical snapshot can verify successfully while no longer being current.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Snapshot {
    state: State,
    tag: [u8; 32],
}
impl Snapshot {
    pub fn run_id(&self) -> RunId {
        self.state.run
    }
    pub fn revision(&self) -> u64 {
        self.state.revision
    }
    pub fn definition_hash(&self) -> DefinitionHash {
        self.state.definition
    }
    pub fn tokens(&self) -> &[Token] {
        &self.state.tokens
    }
    pub fn to_json(&self) -> Result<String> {
        encode_string(self)
    }
    pub fn from_json(json: &str) -> Result<Self> {
        decode(json)
    }
}

/// A single shared secret authenticates tokens and snapshots with domain separation.
/// Provision a random 32-byte secret out of band. Every key holder can mint claims.
/// This type intentionally implements neither Debug nor Serialize.
pub struct Authenticator {
    key: [u8; 32],
}
impl Authenticator {
    pub fn new(key: [u8; 32]) -> Self {
        Self { key }
    }
    fn mac(&self, domain: &[u8], value: &impl Serialize) -> Result<HmacSha256> {
        let mut mac = HmacSha256::new_from_slice(&self.key).expect("32-byte HMAC key");
        mac.update(domain);
        mac.update(&encode(value)?);
        Ok(mac)
    }
    fn sign(&self, domain: &[u8], value: &impl Serialize) -> Result<[u8; 32]> {
        Ok(self.mac(domain, value)?.finalize().into_bytes().into())
    }
    fn verify(&self, domain: &[u8], value: &impl Serialize, tag: &[u8; 32]) -> Result<()> {
        self.mac(domain, value)?
            .verify_slice(tag)
            .map_err(|_| Error::InvalidToken)
    }
}

/// Workflow engine. Independent instances must use the same authoritative store.
/// `granted` is advisory; `execute` rechecks and commits with an atomic CAS.
pub struct Petri<S: Storage> {
    definition: Definition,
    hash: DefinitionHash,
    storage: Shared<S>,
    auth: Shared<Authenticator>,
}
impl<S: Storage> Petri<S> {
    pub fn new(
        definition: Definition,
        storage: Shared<S>,
        auth: Shared<Authenticator>,
    ) -> Result<Self> {
        definition.validate()?;
        let hash = definition.hash()?;
        Ok(Self {
            definition,
            hash,
            storage,
            auth,
        })
    }
    pub fn definition(&self) -> &Definition {
        &self.definition
    }

    /// Creates one initial token. Atomic insert rejects an already-used run ID.
    pub async fn start(&self, run: RunId) -> Result<Token> {
        let mut tokens = self
            .start_marking(run, vec![self.definition.initial])
            .await?;
        Ok(tokens.remove(0))
    }

    // Only the validated workflow compiler may create multiple root inputs.
    pub(crate) async fn start_marking(
        &self,
        run: RunId,
        entries: Vec<PlaceId>,
    ) -> Result<Vec<Token>> {
        if entries.is_empty() || entries.len() > MAX_TOKENS {
            return Err(Error::Limit);
        }
        let mut tokens = Vec::with_capacity(entries.len());
        for (id, place) in entries.into_iter().enumerate() {
            if !self.definition.places.contains(&place) {
                return Err(Error::InvalidInputs);
            }
            tokens.push(self.issue(Claims {
                version: VERSION,
                definition: self.hash,
                run,
                id: id as u64,
                place,
                scope: vec![],
                parents: vec![],
            })?);
        }
        let snapshot = self.seal(State {
            version: VERSION,
            definition: self.hash,
            run,
            revision: 0,
            next_token: tokens.len() as u64,
            next_scope: 1,
            tokens: tokens.clone(),
        })?;
        self.storage
            .compare_and_swap(self.hash, run, None, snapshot)
            .await?;
        Ok(tokens)
    }

    /// Verifies authenticity and definition binding, not current availability.
    pub fn verify_token(&self, token: &Token) -> Result<()> {
        self.auth
            .verify(b"petri-core/token/v2\0", &token.claims, &token.tag)?;
        if token.claims.version != VERSION
            || token.claims.definition != self.hash
            || !self.definition.places.contains(&token.place())
        {
            return Err(Error::InvalidToken);
        }
        Ok(())
    }

    /// Verifies authenticity and structural consistency, not freshness.
    pub fn verify_snapshot(&self, snapshot: &Snapshot) -> Result<()> {
        self.auth
            .verify(b"petri-core/snapshot/v2\0", &snapshot.state, &snapshot.tag)
            .map_err(|_| Error::InvalidSnapshot)?;
        let s = &snapshot.state;
        if s.version != VERSION
            || s.definition != self.hash
            || s.tokens.len() > MAX_TOKENS
            || s.next_scope == 0
            || s.next_token == 0
        {
            return Err(Error::InvalidSnapshot);
        }
        let mut ids = BTreeSet::new();
        for token in &s.tokens {
            self.verify_token(token)?;
            if token.run_id() != s.run
                || token.id() >= s.next_token
                || !ids.insert(token.id())
                || token
                    .scope()
                    .iter()
                    .any(|id| *id == 0 || *id >= s.next_scope)
                || token.parents().iter().any(|id| *id >= token.id())
            {
                return Err(Error::InvalidSnapshot);
            }
        }
        Ok(())
    }

    pub async fn snapshot(&self, run: RunId) -> Result<Snapshot> {
        let snapshot = self
            .storage
            .load(self.hash, run)
            .await?
            .ok_or(Error::NotFound)?;
        self.verify_snapshot(&snapshot)?;
        if snapshot.run_id() != run {
            return Err(Error::InvalidSnapshot);
        }
        Ok(snapshot)
    }

    /// Restore only into an absent run. Never overwrites a newer marking.
    /// Importing old snapshots into an empty store can replay old work: callers
    /// must choose an authoritative checkpoint and preserve run tombstones.
    pub async fn restore(&self, snapshot: Snapshot) -> Result<()> {
        self.verify_snapshot(&snapshot)?;
        self.storage
            .compare_and_swap(self.hash, snapshot.run_id(), None, snapshot)
            .await
    }

    /// Checks selected inputs against the current marking. Does not reserve them.
    pub async fn granted(
        &self,
        run: RunId,
        transition: TransitionId,
        inputs: &[Token],
    ) -> Result<()> {
        let snapshot = self.snapshot(run).await?;
        self.prepare(&snapshot, transition, inputs)?;
        Ok(())
    }

    /// Consumes inputs and inserts authenticated descendants atomically.
    /// A storage conflict is returned to the caller; no automatic business retry.
    pub async fn execute(
        &self,
        run: RunId,
        transition: TransitionId,
        inputs: &[Token],
    ) -> Result<Vec<Token>> {
        let snapshot = self.snapshot(run).await?;
        let (t, mut scope) = self.prepare(&snapshot, transition, inputs)?;
        let count: usize = t.outputs.iter().map(|a| a.weight as usize).sum();
        let mut state = snapshot.state;
        if state.tokens.len() - inputs.len() + count > MAX_TOKENS {
            return Err(Error::Limit);
        }
        state.revision = state.revision.checked_add(1).ok_or(Error::Limit)?;
        if !self.definition.dependency_graph && inputs.len() > 1 {
            scope.pop();
        }
        if !self.definition.dependency_graph && count > 1 {
            scope.push(state.next_scope);
            state.next_scope = state.next_scope.checked_add(1).ok_or(Error::Limit)?;
        }
        let selected: BTreeSet<_> = inputs.iter().map(Token::id).collect();
        state.tokens.retain(|t| !selected.contains(&t.id()));
        let mut parents: Vec<_> = selected.into_iter().collect();
        parents.sort_unstable();
        let mut outputs = Vec::with_capacity(count);
        for arc in &t.outputs {
            for _ in 0..arc.weight {
                let id = state.next_token;
                state.next_token = id.checked_add(1).ok_or(Error::Limit)?;
                outputs.push(self.issue(Claims {
                    version: VERSION,
                    definition: self.hash,
                    run,
                    id,
                    place: arc.place,
                    scope: scope.clone(),
                    parents: parents.clone(),
                })?);
            }
        }
        state.tokens.extend(outputs.iter().cloned());
        let next = self.seal(state)?;
        self.storage
            .compare_and_swap(self.hash, run, Some(next.revision() - 1), next)
            .await?;
        Ok(outputs)
    }

    fn prepare<'a>(
        &'a self,
        snapshot: &Snapshot,
        transition: TransitionId,
        inputs: &[Token],
    ) -> Result<(&'a Transition, Vec<u64>)> {
        let t = self.definition.transition(transition)?;
        let mut actual = BTreeMap::new();
        let mut ids = BTreeSet::new();
        for token in inputs {
            self.verify_token(token)?;
            if token.run_id() != snapshot.run_id()
                || !ids.insert(token.id())
                || !snapshot.tokens().contains(token)
            {
                return Err(Error::InvalidInputs);
            }
            *actual.entry(token.place()).or_insert(0u32) += 1;
        }
        let expected: BTreeMap<_, _> = t.inputs.iter().map(|a| (a.place, a.weight)).collect();
        if actual != expected {
            return Err(Error::InvalidInputs);
        }
        let scope = inputs[0].scope(); // Non-empty input arcs with positive weights.
        if self.definition.dependency_graph {
            return Ok((t, vec![]));
        }
        if inputs.iter().any(|token| token.scope() != scope) {
            return Err(Error::IncompatibleScope);
        }
        Ok((t, scope.to_vec()))
    }
    fn issue(&self, claims: Claims) -> Result<Token> {
        let tag = self.auth.sign(b"petri-core/token/v2\0", &claims)?;
        Ok(Token { claims, tag })
    }
    fn seal(&self, state: State) -> Result<Snapshot> {
        let tag = self.auth.sign(b"petri-core/snapshot/v2\0", &state)?;
        let snapshot = Snapshot { state, tag };
        encode(&snapshot)?;
        Ok(snapshot)
    }
}

fn encode(value: &impl Serialize) -> Result<Vec<u8>> {
    let bytes = serde_json::to_vec(value).map_err(|_| Error::Encoding)?;
    if bytes.len() > MAX_BYTES {
        return Err(Error::Limit);
    }
    Ok(bytes)
}
fn encode_string(value: &impl Serialize) -> Result<String> {
    String::from_utf8(encode(value)?).map_err(|_| Error::Encoding)
}
fn decode<T: serde::de::DeserializeOwned>(json: &str) -> Result<T> {
    if json.len() > MAX_BYTES {
        return Err(Error::Limit);
    }
    serde_json::from_str(json).map_err(|_| Error::Encoding)
}

#[cfg(feature = "tiercache")]
impl<S: Storage> Petri<S> {
    /// Archives an authoritative snapshot under an immutable revision key.
    /// Cache failures cannot authorize execution and never affect the store.
    pub async fn archive(&self, run: RunId, cache: &tiercache::Cache) -> Result<Snapshot> {
        let snapshot = self.snapshot(run).await?;
        cache
            .put(
                &self.archive_key(run, snapshot.revision()),
                snapshot.to_json()?.as_bytes(),
            )
            .await;
        Ok(snapshot)
    }
    /// Returns a verified historical snapshot, NOT a current execution authority.
    pub async fn read_archive(
        &self,
        run: RunId,
        revision: u64,
        cache: &tiercache::Cache,
    ) -> Result<Option<Snapshot>> {
        let Some(bytes) = cache.get(&self.archive_key(run, revision)).await else {
            return Ok(None);
        };
        let json = std::str::from_utf8(&bytes).map_err(|_| Error::Encoding)?;
        let snapshot = Snapshot::from_json(json)?;
        self.verify_snapshot(&snapshot)?;
        if snapshot.run_id() != run || snapshot.revision() != revision {
            return Err(Error::InvalidSnapshot);
        }
        Ok(Some(snapshot))
    }
    fn archive_key(&self, run: RunId, revision: u64) -> String {
        let hash: String = self.hash.iter().map(|b| format!("{b:02x}")).collect();
        format!("petri-core/v2/{hash}/{run}/{revision}")
    }
}

#[cfg(test)]
#[path = "../../tests/support/engine_internal.rs"]
mod tests;

fn is_false(value: &bool) -> bool {
    !value
}
