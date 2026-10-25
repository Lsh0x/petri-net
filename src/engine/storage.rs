use super::{DefinitionHash, Error, Result, RunId, Snapshot};
use std::{collections::HashMap, future::Future, pin::Pin, sync::Mutex};

pub type BoxFuture<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;

/// Authoritative storage contract. Implementations MUST provide linearizable
/// per-run reads and atomic compare-and-swap, including create-if-absent.
/// Preserve completed runs/tombstones: deleting them permits run-ID reuse.
/// A cache's separate get/put operations do NOT satisfy this contract.
pub trait Storage: Send + Sync {
    fn load(
        &self,
        definition: DefinitionHash,
        run: RunId,
    ) -> BoxFuture<'_, Result<Option<Snapshot>>>;
    /// `None`: create only if absent. `Some(r)`: replace only revision r with r+1.
    /// Failure must leave storage unchanged. An error after an uncertain network
    /// commit requires reconciliation by reading the authoritative marking.
    fn compare_and_swap(
        &self,
        definition: DefinitionHash,
        run: RunId,
        expected: Option<u64>,
        next: Snapshot,
    ) -> BoxFuture<'_, Result<()>>;
}

/// Shared in-process backend. Mutex protects the entire compare-and-swap.
/// Not durable, and not shared across processes.
#[derive(Default)]
pub struct MemoryStorage {
    runs: Mutex<HashMap<(DefinitionHash, RunId), Snapshot>>,
}
impl MemoryStorage {
    pub fn new() -> Self {
        Self::default()
    }
}
impl Storage for MemoryStorage {
    fn load(
        &self,
        definition: DefinitionHash,
        run: RunId,
    ) -> BoxFuture<'_, Result<Option<Snapshot>>> {
        Box::pin(async move {
            Ok(self
                .runs
                .lock()
                .map_err(|_| Error::Storage("poisoned mutex".into()))?
                .get(&(definition, run))
                .cloned())
        })
    }
    fn compare_and_swap(
        &self,
        definition: DefinitionHash,
        run: RunId,
        expected: Option<u64>,
        next: Snapshot,
    ) -> BoxFuture<'_, Result<()>> {
        Box::pin(async move {
            if next.definition_hash() != definition || next.run_id() != run {
                return Err(Error::InvalidSnapshot);
            }
            let mut runs = self
                .runs
                .lock()
                .map_err(|_| Error::Storage("poisoned mutex".into()))?;
            let current = runs.get(&(definition, run));
            match (current, expected) {
                (None, None) => {}
                (Some(current), Some(revision))
                    if current.revision() == revision
                        && revision.checked_add(1) == Some(next.revision()) => {}
                _ => return Err(Error::Conflict),
            }
            runs.insert((definition, run), next);
            Ok(())
        })
    }
}

#[cfg(test)]
#[path = "../../tests/support/storage_internal.rs"]
mod tests;
