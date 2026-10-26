use futures::executor::block_on;
use petri_core::engine::*;
use serde_json::json;
use std::sync::{Arc as Shared, Barrier};

fn definition() -> Definition {
    Definition::new(
        vec![0, 1, 2, 3, 4, 5, 6],
        0,
        vec![
            Transition::new(
                0,
                vec![Arc::new(0, 1)],
                vec![Arc::new(1, 1), Arc::new(2, 1)],
            ),
            Transition::new(1, vec![Arc::new(1, 1)], vec![Arc::new(3, 1)]),
            Transition::new(2, vec![Arc::new(2, 1)], vec![Arc::new(4, 1)]),
            Transition::new(
                3,
                vec![Arc::new(3, 1), Arc::new(4, 1)],
                vec![Arc::new(5, 1)],
            ),
            Transition::new(4, vec![Arc::new(5, 1)], vec![Arc::new(0, 1)]),
            Transition::new(5, vec![Arc::new(5, 1)], vec![]),
        ],
    )
    .unwrap()
}
fn engine() -> Petri<MemoryStorage> {
    Petri::new(
        definition(),
        Shared::new(MemoryStorage::new()),
        Shared::new(Authenticator::new([7; 32])),
    )
    .unwrap()
}
async fn fork(p: &Petri<MemoryStorage>, run: u128) -> Vec<Token> {
    let token = p.start(run).await.unwrap();
    p.execute(run, 0, &[token]).await.unwrap()
}
async fn done(p: &Petri<MemoryStorage>, run: u128) -> Vec<Token> {
    let branches = fork(p, run).await;
    let a = p.execute(run, 1, &branches[..1]).await.unwrap();
    let b = p.execute(run, 2, &branches[1..]).await.unwrap();
    [a, b].concat()
}

#[test]
fn fork_join_loop_and_sink_have_exact_markings_and_provenance() {
    block_on(async {
        let p = engine();
        let completed = done(&p, 1).await;
        assert_eq!(completed[0].scope(), completed[1].scope());
        assert_eq!(completed[0].scope().len(), 1);
        let joined = p.execute(1, 3, &completed).await.unwrap();
        assert_eq!(joined[0].place(), 5);
        assert!(joined[0].scope().is_empty());
        assert_eq!(joined[0].parents(), &[3, 4]);
        assert_eq!(p.snapshot(1).await.unwrap().tokens(), joined.as_slice());
        let root = p.execute(1, 4, &joined).await.unwrap();
        let second = p.execute(1, 0, &root).await.unwrap();
        assert_ne!(second[0].scope(), completed[0].scope());
        let a = p.execute(1, 1, &second[..1]).await.unwrap();
        let b = p.execute(1, 2, &second[1..]).await.unwrap();
        let joined = p.execute(1, 3, &[a, b].concat()).await.unwrap();
        assert!(p.execute(1, 5, &joined).await.unwrap().is_empty());
        assert!(p.snapshot(1).await.unwrap().tokens().is_empty());
    });
}

#[test]
fn two_services_recover_workflow_and_exchange_json_tokens() {
    block_on(async {
        let store = Shared::new(MemoryStorage::new());
        let p = Petri::new(
            definition(),
            store.clone(),
            Shared::new(Authenticator::new([7; 32])),
        )
        .unwrap();
        let definition = Definition::from_json(&p.definition().to_json().unwrap()).unwrap();
        let q = Petri::new(definition, store, Shared::new(Authenticator::new([7; 32]))).unwrap();
        let branches = fork(&p, 1).await;
        let token = Token::from_json(&branches[0].to_json().unwrap()).unwrap();
        q.verify_token(&token).unwrap();
        q.granted(1, 1, std::slice::from_ref(&token)).await.unwrap();
        q.execute(1, 1, &[token]).await.unwrap();
        assert_eq!(p.snapshot(1).await.unwrap().revision(), 2);
    });
}

#[test]
fn replay_is_rejected_even_though_old_mac_still_verifies() {
    block_on(async {
        let p = engine();
        let root = p.start(1).await.unwrap();
        p.execute(1, 0, std::slice::from_ref(&root)).await.unwrap();
        p.verify_token(&root).unwrap();
        assert_eq!(
            p.granted(1, 0, std::slice::from_ref(&root)).await,
            Err(Error::InvalidInputs)
        );
        assert_eq!(p.execute(1, 0, &[root]).await, Err(Error::InvalidInputs));
    });
}

#[test]
fn duplicate_run_and_missing_run_fail() {
    block_on(async {
        let p = engine();
        p.start(1).await.unwrap();
        assert_eq!(p.start(1).await, Err(Error::Conflict));
        assert_eq!(p.snapshot(99).await, Err(Error::NotFound));
    });
}

#[test]
fn wrong_transition_missing_duplicate_wrong_place_and_cross_run_inputs_fail_atomically() {
    block_on(async {
        let p = engine();
        let branches = fork(&p, 1).await;
        let others = fork(&p, 2).await;
        let before = p.snapshot(1).await.unwrap();
        assert_eq!(
            p.execute(1, 999, &branches[..1]).await,
            Err(Error::UnknownTransition)
        );
        for inputs in [
            vec![],
            vec![branches[1].clone()],
            vec![branches[0].clone(); 2],
            vec![others[0].clone()],
        ] {
            assert_eq!(p.execute(1, 1, &inputs).await, Err(Error::InvalidInputs));
        }
        assert_eq!(p.execute(1, 3, &branches).await, Err(Error::InvalidInputs));
        assert_eq!(p.snapshot(1).await.unwrap(), before);
    });
}

#[test]
fn every_token_claim_and_tag_are_authenticated() {
    block_on(async {
        let p = engine();
        let token = p.start(1).await.unwrap();
        let original = serde_json::to_value(&token).unwrap();
        for (field, value) in [
            ("version", json!(3)),
            ("run", json!(2)),
            ("id", json!(8)),
            ("place", json!(1)),
            ("scope", json!([1])),
            ("parents", json!([1])),
            ("definition", json!(vec![0; 32])),
        ] {
            let mut value_json = original.clone();
            value_json["claims"][field] = value;
            let forged: Token = serde_json::from_value(value_json).unwrap();
            assert_eq!(p.verify_token(&forged), Err(Error::InvalidToken), "{field}");
        }
        let mut forged = original;
        forged["tag"][0] = json!(255);
        // Ensure the changed byte differs from the original tag.
        forged["tag"][0] = json!(
            (serde_json::to_value(&token).unwrap()["tag"][0]
                .as_u64()
                .unwrap()
                + 1)
                % 256
        );
        assert_eq!(
            p.verify_token(&serde_json::from_value(forged).unwrap()),
            Err(Error::InvalidToken)
        );
    });
}

#[test]
fn wrong_secret_and_wrong_definition_reject_tokens() {
    block_on(async {
        let p = engine();
        let token = p.start(1).await.unwrap();
        let wrong_key = Petri::new(
            definition(),
            Shared::new(MemoryStorage::new()),
            Shared::new(Authenticator::new([8; 32])),
        )
        .unwrap();
        assert_eq!(wrong_key.verify_token(&token), Err(Error::InvalidToken));
        let other = Definition::new(vec![0, 1], 0, vec![]).unwrap();
        let wrong_net = Petri::new(
            other,
            Shared::new(MemoryStorage::new()),
            Shared::new(Authenticator::new([7; 32])),
        )
        .unwrap();
        assert_eq!(wrong_net.verify_token(&token), Err(Error::InvalidToken));
    });
}

#[test]
fn snapshot_export_import_verifies_but_cannot_overwrite_current_state() {
    block_on(async {
        let p = engine();
        let branches = fork(&p, u128::MAX).await;
        let old =
            Snapshot::from_json(&p.snapshot(u128::MAX).await.unwrap().to_json().unwrap()).unwrap();
        p.verify_snapshot(&old).unwrap();
        p.execute(u128::MAX, 1, &branches[..1]).await.unwrap();
        assert_eq!(p.restore(old.clone()).await, Err(Error::Conflict));
        let q = engine();
        q.restore(old.clone()).await.unwrap();
        assert_eq!(q.snapshot(u128::MAX).await.unwrap(), old);
        // Snapshot edits are syntactically valid JSON but fail authentication.
        let json = old
            .to_json()
            .unwrap()
            .replace("\"revision\":1", "\"revision\":0");
        let forged = Snapshot::from_json(&json).unwrap();
        assert_eq!(q.verify_snapshot(&forged), Err(Error::InvalidSnapshot));
    });
}

#[test]
fn weighted_arcs_require_exact_multiplicity() {
    block_on(async {
        let d = Definition::new(
            vec![0, 1, 2],
            0,
            vec![
                Transition::new(0, vec![Arc::new(0, 1)], vec![Arc::new(1, 3)]),
                Transition::new(1, vec![Arc::new(1, 3)], vec![Arc::new(2, 1)]),
            ],
        )
        .unwrap();
        let p = Petri::new(
            d,
            Shared::new(MemoryStorage::new()),
            Shared::new(Authenticator::new([7; 32])),
        )
        .unwrap();
        let root = p.start(1).await.unwrap();
        let tokens = p.execute(1, 0, &[root]).await.unwrap();
        assert_eq!(
            p.granted(1, 1, &tokens[..2]).await,
            Err(Error::InvalidInputs)
        );
        let joined = p.execute(1, 1, &tokens).await.unwrap();
        assert!(joined[0].scope().is_empty());
        assert_eq!(joined[0].parents().len(), 3);
    });
}

#[test]
fn nested_fork_restores_outer_context_before_outer_join() {
    block_on(async {
        let d = Definition::new(
            (0..8).collect(),
            0,
            vec![
                Transition::new(
                    0,
                    vec![Arc::new(0, 1)],
                    vec![Arc::new(1, 1), Arc::new(2, 1)],
                ),
                Transition::new(
                    1,
                    vec![Arc::new(1, 1)],
                    vec![Arc::new(3, 1), Arc::new(4, 1)],
                ),
                Transition::new(
                    2,
                    vec![Arc::new(3, 1), Arc::new(4, 1)],
                    vec![Arc::new(5, 1)],
                ),
                Transition::new(
                    3,
                    vec![Arc::new(5, 1), Arc::new(2, 1)],
                    vec![Arc::new(6, 1)],
                ),
            ],
        )
        .unwrap();
        let p = Petri::new(
            d,
            Shared::new(MemoryStorage::new()),
            Shared::new(Authenticator::new([7; 32])),
        )
        .unwrap();
        let root = p.start(1).await.unwrap();
        let outer = p.execute(1, 0, &[root]).await.unwrap();
        let inner = p.execute(1, 1, &outer[..1]).await.unwrap();
        assert_eq!(inner[0].scope().len(), 2);
        let merged = p.execute(1, 2, &inner).await.unwrap();
        assert_eq!(merged[0].scope(), outer[1].scope());
        p.execute(1, 3, &[merged[0].clone(), outer[1].clone()])
            .await
            .unwrap();
    });
}

struct RaceStore {
    inner: MemoryStorage,
    barrier: Barrier,
}
impl Storage for RaceStore {
    fn load(&self, d: DefinitionHash, r: RunId) -> BoxFuture<'_, Result<Option<Snapshot>>> {
        Box::pin(async move {
            let snapshot = self.inner.load(d, r).await?;
            self.barrier.wait();
            Ok(snapshot)
        })
    }
    fn compare_and_swap(
        &self,
        d: DefinitionHash,
        r: RunId,
        e: Option<u64>,
        n: Snapshot,
    ) -> BoxFuture<'_, Result<()>> {
        self.inner.compare_and_swap(d, r, e, n)
    }
}
#[test]
fn two_real_threads_competing_on_same_revision_only_commit_once() {
    let store = Shared::new(RaceStore {
        inner: MemoryStorage::new(),
        barrier: Barrier::new(2),
    });
    let auth = Shared::new(Authenticator::new([7; 32]));
    let p = Shared::new(Petri::new(definition(), store.clone(), auth).unwrap());
    let token = block_on(p.start(1)).unwrap();
    let handles: Vec<_> = (0..2)
        .map(|_| {
            let p = p.clone();
            let token = token.clone();
            std::thread::spawn(move || block_on(p.execute(1, 0, &[token])))
        })
        .collect();
    let results: Vec<_> = handles.into_iter().map(|h| h.join().unwrap()).collect();
    assert_eq!(results.iter().filter(|r| r.is_ok()).count(), 1);
    assert_eq!(
        results
            .iter()
            .filter(|r| **r == Err(Error::Conflict))
            .count(),
        1
    );
    let snapshot = block_on(store.inner.load(definition().hash().unwrap(), 1))
        .unwrap()
        .unwrap();
    assert_eq!(snapshot.revision(), 1);
    assert_eq!(snapshot.tokens().len(), 2);
}

#[test]
fn definition_validation_rejects_ambiguous_and_unbounded_arcs() {
    for (places, initial, transitions) in [
        (vec![], 0, vec![]),
        (vec![0, 0], 0, vec![]),
        (vec![0], 1, vec![]),
        (vec![0], 0, vec![Transition::new(0, vec![], vec![])]),
        (
            vec![0],
            0,
            vec![Transition::new(0, vec![Arc::new(0, 0)], vec![])],
        ),
        (
            vec![0],
            0,
            vec![Transition::new(0, vec![Arc::new(9, 1)], vec![])],
        ),
        (
            vec![0],
            0,
            vec![Transition::new(
                0,
                vec![Arc::new(0, 1), Arc::new(0, 1)],
                vec![],
            )],
        ),
        (
            vec![0],
            0,
            vec![Transition::new(0, vec![Arc::new(0, 1)], vec![]); 2],
        ),
    ] {
        assert_eq!(
            Definition::new(places, initial, transitions),
            Err(Error::InvalidDefinition)
        );
    }
    assert_eq!(
        Definition::new(
            vec![0],
            0,
            vec![Transition::new(0, vec![Arc::new(0, 5000)], vec![])]
        ),
        Err(Error::Limit)
    );
    assert!(Definition::from_json("{}").is_err());
    let json = definition()
        .to_json()
        .unwrap()
        .replace("\"version\":2", "\"version\":3");
    assert_eq!(Definition::from_json(&json), Err(Error::InvalidDefinition));
    assert!(Token::from_json("{}").is_err());
    assert!(Snapshot::from_json("{}").is_err());
}

#[cfg(feature = "tiercache")]
#[test]
fn immutable_archives_never_authorize_stale_execution() {
    block_on(async {
        let cache = tiercache::Cache::new(vec![Box::new(tiercache::Lru::new(16))]);
        let p = engine();
        assert!(p.read_archive(1, 0, &cache).await.unwrap().is_none());
        let root = p.start(1).await.unwrap();
        let archived = p.archive(1, &cache).await.unwrap();
        p.execute(1, 0, std::slice::from_ref(&root)).await.unwrap();
        assert_eq!(p.read_archive(1, 0, &cache).await.unwrap(), Some(archived));
        assert_eq!(p.granted(1, 0, &[root]).await, Err(Error::InvalidInputs));
    });
}

struct Offline;
impl Storage for Offline {
    fn load(&self, _: DefinitionHash, _: RunId) -> BoxFuture<'_, Result<Option<Snapshot>>> {
        Box::pin(async { Err(Error::Storage("offline".into())) })
    }
    fn compare_and_swap(
        &self,
        _: DefinitionHash,
        _: RunId,
        _: Option<u64>,
        _: Snapshot,
    ) -> BoxFuture<'_, Result<()>> {
        Box::pin(async { Err(Error::Storage("offline".into())) })
    }
}
#[test]
fn backend_failure_is_not_a_cache_miss_or_successful_execution() {
    block_on(async {
        let p = Petri::new(
            definition(),
            Shared::new(Offline),
            Shared::new(Authenticator::new([7; 32])),
        )
        .unwrap();
        assert_eq!(p.start(1).await, Err(Error::Storage("offline".into())));
        assert_eq!(p.snapshot(1).await, Err(Error::Storage("offline".into())));
    });
}

proptest::proptest! {
    #![proptest_config(proptest::test_runner::Config::with_cases(128))]
    #[test]
    fn arbitrary_run_ids_round_trip(run in proptest::prelude::any::<u128>()) {
        block_on(async {
            let p=engine();let token=p.start(run).await.unwrap();
            let transported=Token::from_json(&token.to_json().unwrap()).unwrap();
            p.verify_token(&transported).unwrap();
            assert_eq!(transported,token);
            let snapshot=p.snapshot(run).await.unwrap();
            let restored=Snapshot::from_json(&snapshot.to_json().unwrap()).unwrap();
            p.verify_snapshot(&restored).unwrap();assert_eq!(snapshot,restored);
        });
    }
    #[test]
    fn arbitrary_completion_order_preserves_join_and_rejects_replay(reverse in proptest::prelude::any::<bool>(), run in proptest::prelude::any::<u64>()) {
        block_on(async {
            let p=engine();let branches=fork(&p,run.into()).await;
            let order=if reverse {[1,0]} else {[0,1]};
            let mut completed=Vec::new();
            for index in order {
                let input=&branches[index..index+1];
                let output=p.execute(run.into(), index as u32+1, input).await.unwrap();
                assert_eq!(p.granted(run.into(),index as u32+1,input).await,Err(Error::InvalidInputs));
                completed.extend(output);
            }
            let result=p.execute(run.into(), 3, &completed).await.unwrap();
            assert_eq!(result.len(),1);assert_eq!(result[0].place(),5);
            assert_eq!(p.snapshot(run.into()).await.unwrap().tokens().len(),1);
        });
    }
}

struct MisroutedStore(Snapshot);
impl Storage for MisroutedStore {
    fn load(&self, _: DefinitionHash, _: RunId) -> BoxFuture<'_, Result<Option<Snapshot>>> {
        Box::pin(async { Ok(Some(self.0.clone())) })
    }
    fn compare_and_swap(
        &self,
        _: DefinitionHash,
        _: RunId,
        _: Option<u64>,
        _: Snapshot,
    ) -> BoxFuture<'_, Result<()>> {
        Box::pin(async { Err(Error::Conflict) })
    }
}
#[test]
fn wrongly_routed_storage_record_is_rejected() {
    block_on(async {
        let p = engine();
        p.start(1).await.unwrap();

        assert_eq!(p.definition().transitions().len(), 6);
        let q = Petri::new(
            definition(),
            Shared::new(MisroutedStore(p.snapshot(1).await.unwrap())),
            Shared::new(Authenticator::new([7; 32])),
        )
        .unwrap();
        assert_eq!(q.snapshot(2).await, Err(Error::InvalidSnapshot));
    });
}
