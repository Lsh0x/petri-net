use super::*;
use futures::executor::block_on;

fn engine() -> Petri<MemoryStorage> {
    Petri::new(
        Definition::new(
            vec![0, 1],
            0,
            vec![Transition::new(
                0,
                vec![Arc::new(0, 1)],
                vec![Arc::new(1, 1)],
            )],
        )
        .unwrap(),
        Shared::new(MemoryStorage::new()),
        Shared::new(Authenticator::new([7; 32])),
    )
    .unwrap()
}
#[test]
fn authenticated_but_structurally_invalid_snapshots_are_rejected() {
    block_on(async {
        let p = engine();
        p.start(1, Value::Null).await.unwrap();
        let original = p.snapshot(1).await.unwrap().state;
        let mut cases = Vec::new();
        let mut s = original.clone();
        s.version = 2;
        cases.push(s);
        let mut s = original.clone();
        s.definition = [0; 32];
        cases.push(s);
        let mut s = original.clone();
        s.next_token = 0;
        cases.push(s);
        let mut s = original.clone();
        s.next_scope = 0;
        cases.push(s);
        let mut s = original.clone();
        s.tokens.push(s.tokens[0].clone());
        cases.push(s);
        let mut s = original.clone();
        s.tokens[0].claims.run = 2;
        s.tokens[0] = p.issue(s.tokens[0].claims.clone()).unwrap();
        cases.push(s);
        let mut s = original.clone();
        s.tokens[0].claims.id = 1;
        s.tokens[0] = p.issue(s.tokens[0].claims.clone()).unwrap();
        cases.push(s);
        let mut s = original.clone();
        s.tokens[0].claims.scope = vec![0];
        s.tokens[0] = p.issue(s.tokens[0].claims.clone()).unwrap();
        cases.push(s);
        let mut s = original.clone();
        s.tokens[0].claims.parents = vec![0];
        s.tokens[0] = p.issue(s.tokens[0].claims.clone()).unwrap();
        cases.push(s);
        let mut s = original.clone();
        s.tokens = vec![s.tokens[0].clone(); MAX_TOKENS + 1];
        cases.push(s);
        for state in cases {
            assert_eq!(
                p.verify_snapshot(&p.seal(state).unwrap()),
                Err(Error::InvalidSnapshot)
            );
        }
        for (version, place) in [(2, 0), (1, 99)] {
            let mut claims = original.tokens[0].claims.clone();
            claims.version = version;
            claims.place = place;
            assert_eq!(
                p.verify_token(&p.issue(claims).unwrap()),
                Err(Error::InvalidToken)
            );
        }
    });
}

#[test]
fn scope_mismatch_and_counter_exhaustion_never_commit() {
    block_on(async {
        let p = engine();
        p.start(1, Value::Null).await.unwrap();
        let mut state = p.snapshot(1).await.unwrap().state;
        state.revision = u64::MAX;
        let snapshot = p.seal(state).unwrap();
        let q = engine();
        q.restore(snapshot.clone()).await.unwrap();
        assert_eq!(
            q.execute(1, 0, snapshot.tokens(), vec![Value::Null]).await,
            Err(Error::Limit)
        );
        assert_eq!(q.snapshot(1).await.unwrap(), snapshot);
        let mut state = snapshot.state;
        state.revision = 0;
        state.next_token = u64::MAX;
        let q = engine();
        let snapshot = p.seal(state).unwrap();
        q.restore(snapshot.clone()).await.unwrap();
        assert_eq!(
            q.execute(1, 0, snapshot.tokens(), vec![Value::Null]).await,
            Err(Error::Limit)
        );
        assert_eq!(q.snapshot(1).await.unwrap(), snapshot);
    });
}

#[test]
fn incompatible_forks_and_scope_overflow_fail() {
    block_on(async {
        let d = Definition::new(
            vec![0, 1],
            0,
            vec![
                Transition::new(0, vec![Arc::new(0, 2)], vec![Arc::new(1, 1)]),
                Transition::new(1, vec![Arc::new(0, 1)], vec![Arc::new(1, 2)]),
            ],
        )
        .unwrap();
        let p = Petri::new(
            d.clone(),
            Shared::new(MemoryStorage::new()),
            Shared::new(Authenticator::new([7; 32])),
        )
        .unwrap();
        let token = p.start(1, Value::Null).await.unwrap();
        let mut state = p.snapshot(1).await.unwrap().state;
        let mut a = token.claims.clone();
        a.id = 1;
        a.scope = vec![1];
        let mut b = token.claims.clone();
        b.id = 2;
        b.scope = vec![2];
        state.tokens = vec![p.issue(a).unwrap(), p.issue(b).unwrap()];
        state.next_token = 3;
        state.next_scope = 3;
        let q = Petri::new(
            d.clone(),
            Shared::new(MemoryStorage::new()),
            Shared::new(Authenticator::new([7; 32])),
        )
        .unwrap();
        let snapshot = p.seal(state).unwrap();
        q.restore(snapshot.clone()).await.unwrap();
        assert_eq!(
            q.granted(1, 0, snapshot.tokens()).await,
            Err(Error::IncompatibleScope)
        );
        let mut state = snapshot.state;
        state.next_scope = u64::MAX;
        let q = Petri::new(
            d,
            Shared::new(MemoryStorage::new()),
            Shared::new(Authenticator::new([7; 32])),
        )
        .unwrap();
        let snapshot = p.seal(state).unwrap();
        q.restore(snapshot.clone()).await.unwrap();
        assert_eq!(
            q.execute(1, 1, &snapshot.tokens()[..1], vec![Value::Null; 2])
                .await,
            Err(Error::Limit)
        );
    });
}

#[test]
fn size_limits_and_error_display() {
    block_on(async {
        let p = engine();
        let huge = Value::String("x".repeat(MAX_BYTES));
        assert_eq!(p.start(1, huge.clone()).await, Err(Error::Limit));
        assert_eq!(p.snapshot(1).await, Err(Error::NotFound));
        let root = p.start(1, Value::Null).await.unwrap();
        let before = p.snapshot(1).await.unwrap();
        assert_eq!(
            p.execute(1, 0, &[root], vec![huge]).await,
            Err(Error::Limit)
        );
        assert_eq!(p.snapshot(1).await.unwrap(), before);
        assert_eq!(
            Token::from_json(&" ".repeat(MAX_BYTES + 1)),
            Err(Error::Limit)
        );
        assert_eq!(format!("{}", Error::Conflict), "Conflict");
        fn error_trait(_: &dyn std::error::Error) {}
        error_trait(&Error::Encoding);
    });
}

#[test]
fn full_marking_rejects_growth_without_modification() {
    block_on(async {
        let d = Definition::new(
            vec![0],
            0,
            vec![Transition::new(
                0,
                vec![Arc::new(0, 1)],
                vec![Arc::new(0, 2)],
            )],
        )
        .unwrap();
        let p = Petri::new(
            d,
            Shared::new(MemoryStorage::new()),
            Shared::new(Authenticator::new([7; 32])),
        )
        .unwrap();
        p.start(1, Value::Null).await.unwrap();
        let mut state = p.snapshot(1).await.unwrap().state;
        let root = state.tokens[0].claims.clone();
        state.tokens = (0..MAX_TOKENS as u64)
            .map(|id| {
                let mut c = root.clone();
                c.id = id;
                p.issue(c).unwrap()
            })
            .collect();
        state.next_token = MAX_TOKENS as u64;
        let q = Petri::new(
            p.definition.clone(),
            Shared::new(MemoryStorage::new()),
            p.auth.clone(),
        )
        .unwrap();
        let snapshot = p.seal(state).unwrap();
        q.restore(snapshot.clone()).await.unwrap();
        assert_eq!(
            q.execute(1, 0, &snapshot.tokens()[..1], vec![Value::Null; 2])
                .await,
            Err(Error::Limit)
        );
        assert_eq!(q.snapshot(1).await.unwrap(), snapshot);
    });
}

#[cfg(feature = "tiercache")]
#[test]
fn corrupted_archives_and_misrouted_revisions_are_rejected() {
    block_on(async {
        let cache = tiercache::Cache::new(vec![Box::new(tiercache::Lru::new(16))]);
        let p = engine();
        p.start(1, Value::Null).await.unwrap();
        let snapshot = p.snapshot(1).await.unwrap();
        let key = p.archive_key(1, 99);
        cache
            .put(&key, snapshot.to_json().unwrap().as_bytes())
            .await;
        assert_eq!(
            p.read_archive(1, 99, &cache).await,
            Err(Error::InvalidSnapshot)
        );
        cache.put(&key, &[255]).await;
        assert_eq!(p.read_archive(1, 99, &cache).await, Err(Error::Encoding));
        cache.put(&key, b"{}").await;
        assert_eq!(p.read_archive(1, 99, &cache).await, Err(Error::Encoding));
        let mut forged = snapshot.clone();
        forged.tag[0] ^= 1;
        cache.put(&key, forged.to_json().unwrap().as_bytes()).await;
        assert_eq!(
            p.read_archive(1, 99, &cache).await,
            Err(Error::InvalidSnapshot)
        );
    });
}
