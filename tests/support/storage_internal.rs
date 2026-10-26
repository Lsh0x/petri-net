use super::*;
use crate::engine::{Authenticator, Definition, Petri};
use futures::executor::block_on;
use std::sync::Arc;
#[test]
fn backend_rejects_namespace_mismatch_and_invalid_revision_update() {
    block_on(async {
        let d = Definition::new(vec![0], 0, vec![]).unwrap();
        let store = Arc::new(MemoryStorage::new());
        let p = Petri::new(
            d.clone(),
            store.clone(),
            Arc::new(Authenticator::new([7; 32])),
        )
        .unwrap();
        p.start(1).await.unwrap();
        let snapshot = p.snapshot(1).await.unwrap();
        assert_eq!(
            store
                .compare_and_swap(d.hash().unwrap(), 2, None, snapshot.clone())
                .await,
            Err(Error::InvalidSnapshot)
        );
        assert_eq!(
            store
                .compare_and_swap([0; 32], 1, None, snapshot.clone())
                .await,
            Err(Error::InvalidSnapshot)
        );
        assert_eq!(
            store
                .compare_and_swap(d.hash().unwrap(), 1, Some(0), snapshot)
                .await,
            Err(Error::Conflict)
        );
        assert!(store.load(d.hash().unwrap(), 99).await.unwrap().is_none());
    });
}
#[test]
fn poisoned_storage_fails_closed() {
    let store = MemoryStorage::new();
    let _ = std::panic::catch_unwind(|| {
        let _guard = store.runs.lock().unwrap();
        panic!("poison");
    });
    assert!(matches!(
        block_on(store.load([0; 32], 1)),
        Err(Error::Storage(_))
    ));
}
