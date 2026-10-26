use futures::executor::block_on;
use petri_core::engine::{Authenticator, MemoryStorage};
use petri_core::workflow::{Error, Petri, TaskToken, Workflow};
use serde_json::{json, Value};
use std::sync::Arc;

const ORDER: &str = include_str!("../examples/workflows/order.yaml");
const MEDIA: &str = include_str!("../examples/workflows/media.yaml");
const DEPLOY: &str = include_str!("../examples/workflows/deploy.yaml");
fn make_engine(yaml: &str) -> Petri<MemoryStorage> {
    Petri::new(
        Workflow::from_yaml(yaml).unwrap(),
        Arc::new(MemoryStorage::new()),
        Arc::new(Authenticator::new([7; 32])),
    )
    .unwrap()
}
fn ids(tokens: &[TaskToken]) -> Vec<u32> {
    tokens.iter().map(TaskToken::task_id).collect()
}
async fn finish(p: &Petri<MemoryStorage>, run: u128, id: u32) {
    let token = p
        .ready(run)
        .await
        .unwrap()
        .into_iter()
        .find(|t| t.task_id() == id)
        .unwrap();
    p.granted(&token).await.unwrap();
    let receipt = p.executed(token, json!({"task":id})).await.unwrap();
    assert_eq!(receipt.task_id(), id);
    assert_eq!(receipt.run_id(), run);
    assert_eq!(receipt.result(), &json!({"task":id}));
    p.verify_completion(&receipt).unwrap();
}

#[test]
fn order_shared_payment_unlocks_invoice_and_preparation_without_stealing_tokens() {
    block_on(async {
        let p = make_engine(ORDER);
        let roots = p.start(1, json!("request")).await.unwrap();
        assert_eq!(ids(&roots), vec![1]);
        assert_eq!(roots[0].initial_data(), Some(&json!("request")));
        finish(&p, 1, 1).await;
        assert_eq!(ids(&p.ready(1).await.unwrap()), vec![2, 3, 4]);
        finish(&p, 1, 2).await;
        assert_eq!(ids(&p.ready(1).await.unwrap()), vec![3, 4, 5]);
        finish(&p, 1, 5).await; // Invoice consumes its own payment edge only.
        finish(&p, 1, 3).await;
        assert_eq!(ids(&p.ready(1).await.unwrap()), vec![4, 6]);
        let prep = p
            .ready(1)
            .await
            .unwrap()
            .into_iter()
            .find(|t| t.task_id() == 6)
            .unwrap();
        assert_eq!(prep.dependencies(), &[2, 3]);
        assert_eq!(prep.input(2), Some(&json!({"task":2})));
        assert_eq!(prep.input(3), Some(&json!({"task":3})));
        assert!(prep.initial_data().is_none());
        assert!(prep.input(999).is_none());
        finish(&p, 1, 6).await;
        assert_eq!(ids(&p.ready(1).await.unwrap()), vec![4]);
        finish(&p, 1, 4).await;
        finish(&p, 1, 7).await;
        finish(&p, 1, 8).await;
        let status = p.status(1).await.unwrap();
        assert!(status.finished);
        assert!(status.ready.is_empty());
        assert_eq!(status.completed, (1..=8).collect::<Vec<_>>());
        assert_eq!(status.revision, 8);
    });
}

#[test]
fn crossed_media_joins_do_not_require_matching_fork_stacks() {
    block_on(async {
        let p = make_engine(MEDIA);
        p.start(1, Value::Null).await.unwrap();
        for id in [1, 2, 3, 4, 5, 6] {
            finish(&p, 1, id).await;
        }
        assert_eq!(ids(&p.ready(1).await.unwrap()), vec![7, 8]);
        finish(&p, 1, 8).await;
        finish(&p, 1, 10).await; // Publish while gallery is blocked.
        assert_eq!(ids(&p.ready(1).await.unwrap()), vec![7]);
        finish(&p, 1, 7).await;
        finish(&p, 1, 9).await;
        finish(&p, 1, 11).await;
        assert!(p.status(1).await.unwrap().finished);
    });
}

#[test]
fn independent_roots_are_created_atomically_and_terminals_all_count() {
    block_on(async {
        let p = make_engine(DEPLOY);
        let roots = p.start(u128::MAX, json!(42)).await.unwrap();
        assert_eq!(ids(&roots), vec![1, 2, 3]);
        assert_eq!(p.status(u128::MAX).await.unwrap().revision, 0);
        assert!(p.start(u128::MAX, Value::Null).await.is_err());
        for id in [2, 3, 5, 1, 4, 6, 8, 9] {
            finish(&p, u128::MAX, id).await;
        }
        assert_eq!(ids(&p.ready(u128::MAX).await.unwrap()), vec![7]);
        finish(&p, u128::MAX, 7).await;
        finish(&p, u128::MAX, 10).await;
        assert!(p.status(u128::MAX).await.unwrap().finished);
        let independent = make_engine("Version: 1\nTasks:\n  A: {Id: 0}\n  B: {Id: 4294967295}\n");
        independent.start(1, Value::Null).await.unwrap();
        finish(&independent, 1, 0).await;
        assert!(!independent.status(1).await.unwrap().finished);
        finish(&independent, 1, u32::MAX).await;
        assert!(independent.status(1).await.unwrap().finished);
    });
}

#[test]
fn yaml_json_export_order_and_dependency_order_compile_to_same_definition() {
    block_on(async {
        let workflow = Workflow::from_yaml(ORDER).unwrap();
        assert_eq!(
            Workflow::from_json(&workflow.to_json().unwrap()).unwrap(),
            workflow
        );
        assert_eq!(
            Workflow::from_yaml(&workflow.to_yaml().unwrap()).unwrap(),
            workflow
        );
        assert_eq!(workflow.task_id("Paiement"), Some(2));
        assert_eq!(workflow.task_name(2), Some("Paiement"));
        assert_eq!(workflow.task_name(99), None);
        assert_eq!(workflow.task_id("absent"), None);
        assert!(workflow.description(2).is_none());
        let a = Workflow::from_yaml(
            "Version: 1\nTasks:\n  Z: {Id: 1}\n  A: {Id: 2}\n  B: {Id: 3, DependsOn: [Z, A]}\n",
        )
        .unwrap();
        let b = Workflow::from_yaml(
            "Tasks:\n  B: {DependsOn: [A, Z], Id: 3}\n  A: {Id: 2}\n  Z: {Id: 1}\nVersion: 1\n",
        )
        .unwrap();
        let pa = Petri::new(
            a,
            Arc::new(MemoryStorage::new()),
            Arc::new(Authenticator::new([7; 32])),
        )
        .unwrap();
        let pb = Petri::new(
            b,
            Arc::new(MemoryStorage::new()),
            Arc::new(Authenticator::new([7; 32])),
        )
        .unwrap();
        pa.start(1, Value::Null).await.unwrap();
        pb.start(1, Value::Null).await.unwrap();
        assert_eq!(
            pa.snapshot(1).await.unwrap().definition_hash(),
            pb.snapshot(1).await.unwrap().definition_hash()
        );
    });
}

#[test]
fn services_transport_tickets_restore_snapshots_and_reject_replays() {
    block_on(async {
        let workflow = Workflow::from_yaml(ORDER).unwrap();
        let store = Arc::new(MemoryStorage::new());
        let auth = Arc::new(Authenticator::new([7; 32]));
        let p = Petri::new(workflow.clone(), store.clone(), auth.clone()).unwrap();
        let q = Petri::new(workflow.clone(), store, auth.clone()).unwrap();
        let root = p.start(1, Value::Null).await.unwrap().remove(0);
        let ticket = TaskToken::from_json(&root.to_json().unwrap()).unwrap();
        q.granted(&ticket).await.unwrap();
        q.executed(ticket, Value::Null).await.unwrap();
        assert!(p.granted(&root).await.is_err());
        assert!(p.executed(root, Value::Null).await.is_err());
        let recovered = Petri::new(workflow, Arc::new(MemoryStorage::new()), auth).unwrap();
        recovered
            .restore(p.snapshot(1).await.unwrap())
            .await
            .unwrap();
        assert_eq!(ids(&recovered.ready(1).await.unwrap()), vec![2, 3, 4]);
        assert!(recovered
            .restore(p.snapshot(1).await.unwrap())
            .await
            .is_err());
    });
}

#[test]
fn forged_task_run_dependency_payload_and_receipt_are_rejected() {
    block_on(async {
        let p = make_engine(ORDER);
        let root = p.start(1, Value::Null).await.unwrap().remove(0);
        p.start(2, Value::Null).await.unwrap();
        let original = serde_json::to_value(&root).unwrap();
        for (field, value) in [
            ("task", json!(999)),
            ("task", json!(2)),
            ("run", json!(2)),
            ("dependencies", json!([1])),
            ("inputs", json!([])),
        ] {
            let mut forged = original.clone();
            forged[field] = value;
            let ticket: TaskToken = serde_json::from_value(forged).unwrap();
            assert!(p.granted(&ticket).await.is_err());
            assert!(p.executed(ticket, Value::Null).await.is_err());
        }
        let mut forged = original;
        forged["inputs"][0]["claims"]["data"] = json!("forged");
        let ticket: TaskToken = serde_json::from_value(forged).unwrap();
        assert!(p.granted(&ticket).await.is_err());
        let receipt = p.executed(root, Value::Null).await.unwrap();
        let mut forged = serde_json::to_value(&receipt).unwrap();
        forged["task"] = json!(2);
        assert!(p
            .verify_completion(&serde_json::from_value(forged).unwrap())
            .is_err());
        let mut forged = serde_json::to_value(&receipt).unwrap();
        forged["task"] = json!(999);
        assert_eq!(
            p.verify_completion(&serde_json::from_value(forged).unwrap()),
            Err(Error::UnknownTask(999))
        );
        assert!(TaskToken::from_json("{}").is_err());
        let invalid: TaskToken =
            serde_json::from_value(json!({"run":1,"task":2,"dependencies":[1],"inputs":[]}))
                .unwrap();
        assert!(invalid.input(1).is_none());
    });
}

#[test]
fn invalid_graphs_and_silent_typos_are_rejected() {
    for yaml in [
        "Version: 2\nTasks: {A: {Id: 1}}",
        "Version: 1\nTasks: {}",
        "Version: 1\nTasks: {'': {Id: 1}}",
        "Version: 1\nTasks: {A: {Id: 1}, B: {Id: 1}}",
        "Version: 1\nTasks: {A: {Id: 1, DependsOn: Missing}}",
        "Version: 1\nTasks: {A: {Id: 1, DependsOn: A}}",
        "Version: 1\nTasks: {A: {Id: 1, DependsOn: B}, B: {Id: 2, DependsOn: A}}",
        "Version: 1\nTasks: {Root: {Id: 0}, A: {Id: 1, DependsOn: B}, B: {Id: 2, DependsOn: A}}",
        "Version: 1\nTasks: {A: {Id: 1}, B: {Id: 2, DependsOn: [A, A]}}",
        "Version: 1\nTasks: {A: {Id: 1, DependOn: Missing}}",
        "Version: 1\nTasks: {A: {Id: 1}, A: {Id: 2}}",
        "Version: 1\nTasks: {A: {Id: -1}}",
        "Version: 1\nTasks: {A: {Id: 1}}\nUnknown: true",
        "Version: 1\nTasks: {A: {Id: 1}}\n---\nVersion: 1\nTasks: {B: {Id: 2}}",
    ] {
        assert!(Workflow::from_yaml(yaml).is_err(), "{yaml}");
    }
    assert!(Workflow::from_json(r#"{"Version":1,"Tasks":{"A":{"Id":1},"A":{"Id":2}}}"#).is_err());
    assert!(Workflow::from_json("garbage").is_err());
    assert!(Workflow::from_yaml(&" ".repeat(4 * 1024 * 1024 + 1)).is_err());
    let oversized = format!(
        "Version: 1\nTasks:\n{}",
        (0..1025)
            .map(|i| format!("  T{i}: {{Id: {i}}}\n"))
            .collect::<String>()
    );
    assert!(Workflow::from_yaml(&oversized).is_err());
}

#[test]
fn descriptions_are_display_only_and_errors_are_readable() {
    let workflow =
        Workflow::from_yaml("Version: 1\nTasks:\n  A: {Id: 1, Description: Hello}\n").unwrap();
    assert_eq!(workflow.description(1), Some("Hello"));
    assert_eq!(format!("{}", Error::Cycle), "Cycle");
    let _: &dyn std::error::Error = &Error::Cycle;
}

proptest::proptest! {
    #![proptest_config(proptest::test_runner::Config::with_cases(256))]
    #[test]
    fn random_dags_match_reference_scheduler(
        count in 1usize..16,
        edge_bits in proptest::collection::vec(proptest::prelude::any::<bool>(), 225),
        choices in proptest::collection::vec(proptest::prelude::any::<u8>(), 16)
    ) {
        block_on(async {
            let mut tasks=serde_json::Map::new();
            let mut dependencies=Vec::new();
            for child in 0..count {
                let deps:Vec<_>=(0..child).filter(|parent|edge_bits[parent*15+child]).collect();
                let names:Vec<_>=deps.iter().map(|p|format!("T{p}")).collect();
                tasks.insert(format!("T{child}"),json!({"Id":child as u32,"DependsOn":names}));
                dependencies.push(deps);
            }
            let workflow=Workflow::from_json(&json!({"Version":1,"Tasks":tasks}).to_string()).unwrap();
            let workflow=Workflow::from_yaml(&workflow.to_yaml().unwrap()).unwrap();
            let p=Petri::new(workflow,Arc::new(MemoryStorage::new()),Arc::new(Authenticator::new([7;32]))).unwrap();
            p.start(1,json!("root")).await.unwrap();
            let mut completed=std::collections::BTreeSet::new();
            for choice in choices.iter().take(count) {
                let status=p.status(1).await.unwrap();
                let expected:Vec<_>=(0..count).filter(|i|!completed.contains(i)&&dependencies[*i].iter().all(|d|completed.contains(d))).map(|i|i as u32).collect();
                assert_eq!(ids(&status.ready),expected);
                assert!(!status.finished);
                let ticket=status.ready[*choice as usize%status.ready.len()].clone();
                let task=ticket.task_id();
                p.executed(ticket,json!({"task":task})).await.unwrap();
                completed.insert(task as usize);
            }
            let status=p.status(1).await.unwrap();assert!(status.finished);assert!(status.ready.is_empty());assert_eq!(status.completed.len(),count);assert_eq!(status.revision,count as u64);
        });
    }
}

#[test]
fn dense_graphs_exceeding_compilation_budget_fail_before_execution() {
    let tasks: serde_json::Map<_, _> = (0..100)
        .map(|i| {
            (
                format!("T{i}"),
                json!({"Id":i,"DependsOn":(0..i).map(|j|format!("T{j}")).collect::<Vec<_>>()}),
            )
        })
        .collect();
    assert_eq!(
        Workflow::from_json(&json!({"Version":1,"Tasks":tasks}).to_string()),
        Err(Error::Limit)
    );
}

#[cfg(feature = "tiercache")]
#[test]
fn workflow_archives_are_historical_and_cannot_resurrect_a_task() {
    block_on(async {
        let p = make_engine(ORDER);
        let cache = tiercache::Cache::new(vec![Box::new(tiercache::Lru::new(16))]);
        assert!(p.read_archive(1, 0, &cache).await.unwrap().is_none());
        let root = p.start(1, Value::Null).await.unwrap().remove(0);
        let archived = p.archive(1, &cache).await.unwrap();
        p.executed(root.clone(), Value::Null).await.unwrap();
        assert_eq!(p.read_archive(1, 0, &cache).await.unwrap(), Some(archived));
        assert!(p.granted(&root).await.is_err());
    });
}

struct RacingStore {
    inner: MemoryStorage,
    gate: std::sync::atomic::AtomicBool,
    barrier: std::sync::Barrier,
}
impl petri_core::engine::Storage for RacingStore {
    fn load(
        &self,
        d: petri_core::engine::DefinitionHash,
        r: u128,
    ) -> petri_core::engine::BoxFuture<
        '_,
        petri_core::engine::Result<Option<petri_core::engine::Snapshot>>,
    > {
        use petri_core::engine::Storage;
        Box::pin(async move {
            let snapshot = Storage::load(&self.inner, d, r).await?;
            if self.gate.load(std::sync::atomic::Ordering::SeqCst) {
                self.barrier.wait();
            }
            Ok(snapshot)
        })
    }
    fn compare_and_swap(
        &self,
        d: petri_core::engine::DefinitionHash,
        r: u128,
        e: Option<u64>,
        n: petri_core::engine::Snapshot,
    ) -> petri_core::engine::BoxFuture<'_, petri_core::engine::Result<()>> {
        petri_core::engine::Storage::compare_and_swap(&self.inner, d, r, e, n)
    }
}
#[test]
fn racing_siblings_conflict_safely_and_losing_ticket_can_be_retried() {
    let store = Arc::new(RacingStore {
        inner: MemoryStorage::new(),
        gate: std::sync::atomic::AtomicBool::new(false),
        barrier: std::sync::Barrier::new(2),
    });
    let p = Arc::new(
        Petri::new(
            Workflow::from_yaml("Version: 1\nTasks: {A: {Id: 1}, B: {Id: 2}}\n").unwrap(),
            store.clone(),
            Arc::new(Authenticator::new([7; 32])),
        )
        .unwrap(),
    );
    let roots = block_on(p.start(1, Value::Null)).unwrap();
    store.gate.store(true, std::sync::atomic::Ordering::SeqCst);
    let handles: Vec<_> = roots
        .into_iter()
        .map(|ticket| {
            let p = p.clone();
            std::thread::spawn(move || {
                let result = block_on(p.executed(ticket.clone(), Value::Null));
                (ticket, result)
            })
        })
        .collect();
    let results: Vec<_> = handles.into_iter().map(|h| h.join().unwrap()).collect();
    store.gate.store(false, std::sync::atomic::Ordering::SeqCst);
    assert_eq!(results.iter().filter(|(_, r)| r.is_ok()).count(), 1);
    for (ticket, result) in results {
        if result.is_err() {
            assert_eq!(
                result,
                Err(Error::Engine(petri_core::engine::Error::Conflict))
            );
            block_on(p.executed(ticket, Value::Null)).unwrap();
        }
    }
    let status = block_on(p.status(1)).unwrap();
    assert!(status.finished);
    assert_eq!(status.completed, vec![1, 2]);
    assert_eq!(status.revision, 2);
}
