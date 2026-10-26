# Low-level authenticated Petri engine

For YAML/JSON task graphs and business-function examples, start with the main
README. This API is for manually defined numeric, weighted Petri nets.

```rust
use petri_core::engine::{Arc, Authenticator, Definition, MemoryStorage, Petri, Transition};
use std::sync::Arc as Shared;

// Run inside your async runtime.
async fn example() -> Result<(), petri_core::engine::Error> {
    let graph = Definition::new(vec![0, 1], 0, vec![
        Transition::new(1, vec![Arc::new(0, 1)], vec![Arc::new(1, 1)]),
    ])?;
    let p = Petri::new(graph, Shared::new(MemoryStorage::new()),
        Shared::new(Authenticator::new([7; 32])))?; // Demo key only.
    let root = p.start(42).await?;
    p.granted(42, 1, std::slice::from_ref(&root)).await?;
    // Business work, external to the engine.
    let outputs = p.execute(42, 1, &[root]).await?;
    assert_eq!(outputs[0].place(), 1);
    Ok(())
}
```

Tokens carry numeric identities, positions, parent IDs, fork context and an
HMAC proof, never business payloads. Transition output tokens are computed from
the definition; callers do not supply an output-data vector.

Default low-level correlation uses structured fork scopes: multi-output firing
opens a scope, multi-input firing requires identical scopes and closes one.
Nested structured joins and repeated loop occurrences are supported. Empty
outputs are sinks; empty inputs are rejected. The workflow compiler uses a
separate authenticated dependency-edge policy for validated DAGs.

`Storage::load` must be authoritative and per-run CAS must be atomic.
`MemoryStorage` uses a mutex and is neither durable nor shared across processes.
HMAC authenticity does not provide freshness, business-effect idempotence or
protection against a trusted key holder or rollback by the authoritative store.

`Definition::from_json`, token/snapshot serialization and authenticated restore
are available. Deserialization is not authentication. Restore inserts only into
an absent run, so recovery must preserve history and use a trusted checkpoint.
The proof format is version 2; 0.2/0.3 payload-bearing credentials are rejected.
Optional tiercache methods archive immutable revisions, never authorization state.
