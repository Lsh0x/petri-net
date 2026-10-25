# petri-core

Generic Rust building blocks for state-machine Petri nets, hosted in the
`petri-net` repository. Places, transitions and tokens carry your own metadata;
the core has no domain-specific fields or runtime dependencies beyond serde.

Each transition has **one input and one output**. This initial model does not
implement general multi-input/multi-output Petri nets, weighted arcs, guards,
scheduling or automatic firing.

## Example

```rust
use petri_core::{Definition, Marking, Place, Token, Transition};

let net = Definition::new(
    vec![Place::new("pending", ()), Place::new("done", ())],
    vec![Transition::new("complete", "pending", "done", ())],
);
let mut marking = Marking::new();
marking.put_token("pending".into(), Token::new(1, "pending", "job"));

assert_eq!(net.enabled_transitions(&marking).len(), 1);
marking.move_token(&"pending".into(), "done".into(), 1).unwrap();
assert!(net.enabled_transitions(&marking).is_empty());
```

## Modules

- `place`: `Place<P>`, `PlaceId = String`
- `transition`: `Transition<T>`, `TransId = String`
- `token`: `Token<D>`
- `marking`: insertion, lookup, removal and token movement
- `definition`: definition lookup and pure enabled-transition queries

All model types support serde serialization. Token IDs are caller-managed.
Insertion normalizes a token's place ID to its bucket. Definition construction
does not validate IDs or endpoints, and deserialization assumes consistent input.
Enabling checks source occupancy only; applications evaluate their own guards.
Moving a token returns a cloned snapshot and therefore requires `D: Clone`.

## Development

```sh
cargo test --all-targets
cargo test --doc
cargo clippy --all-targets -- -D warnings
cargo run --example workflow
```

This crate has not been published to crates.io. Package-name availability and
licensing must be resolved before publication.
