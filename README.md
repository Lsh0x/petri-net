# petri-core

[![CI](https://github.com/Lsh0x/petri-net/actions/workflows/ci.yml/badge.svg)](https://github.com/Lsh0x/petri-net/actions/workflows/ci.yml)

**Generic places, transitions, tokens and markings for Rust.**

A small, synchronous data model for **state-machine Petri nets**.
Attach your own metadata to places, transitions and tokens, inspect enabled
transitions, and move tokens explicitly.

> Initial development: the package is named `petri-core`, while this repository
> is named `petri-net`. It has not been published to crates.io.

## Installation

Use the Git repository until a crates.io release is available:

```toml
[dependencies]
petri-core = { git = "https://github.com/Lsh0x/petri-net" }
```

The Rust import is `petri_core`. Serde support is always enabled; there are
currently no optional feature flags.

## What it does

The core stores the net's definition and current marking. Your application
provides the domain meaning: a place might represent a workflow stage, a
transition an allowed action, and a token an individual job.

Each transition connects **one source place to one destination place**.
It is enabled when its source contains at least one token. Querying enabled
transitions is pure: it neither consumes tokens nor evaluates application rules.

This is the state-machine subset of Petri nets. Multi-input joins, multi-output
forks, weighted arcs and automatic firing are outside the current API.

## Quick start

```rust
use petri_core::{Definition, Marking, Place, Token, Transition};

let net = Definition::new(
    vec![Place::new("pending", ()), Place::new("done", ())],
    vec![Transition::new("complete", "pending", "done", ())],
);

let pending = "pending".to_string();
let done = "done".to_string();
let mut marking = Marking::new();
marking.put_token(pending.clone(), Token::new(1, pending.clone(), "job-001"));

assert_eq!(net.enabled_transitions(&marking)[0].id, "complete");

let moved = marking.move_token(&pending, done.clone(), 1).unwrap();
assert_eq!(moved.data, "job-001");
assert_eq!(moved.place_id, done);
assert!(marking.tokens_in(&pending).is_empty());
assert!(net.enabled_transitions(&marking).is_empty());
```

Run the checked-in example with `cargo run --example workflow`.

## Bring your own data

The three payload types are independent and default to `()`:

```rust
use petri_core::{Place, Token, Transition};

struct Stage {
    label: String,
}

let pending = Place::new("pending", Stage { label: "Pending review".into() });
let approve = Transition::new("approve", "pending", "approved", "reviewer");
let job = Token::new(42, pending.id.clone(), vec!["document.pdf"]);

assert_eq!(pending.data.label, "Pending review");
assert_eq!(approve.data, "reviewer");
assert_eq!(job.data, vec!["document.pdf"]);
```

Payloads need no trait bounds for construction, insertion, removal or queries.
Cloning, equality, hashing and serialization require the corresponding traits
on the payload when those operations are used. `move_token` requires
`D: Clone` because it returns a snapshot while retaining the moved token.

## Core API

| Type | Responsibility |
| --- | --- |
| `Place<P = ()>` | String ID and place metadata |
| `Transition<T = ()>` | String ID, source, destination and transition metadata |
| `Token<D = ()>` | Caller-assigned `u64` ID, place ID and payload |
| `Marking<D = ()>` | Tokens grouped by place |
| `Definition<P = (), T = ()>` | Places, transitions and lookup/query methods |

`PlaceId` and `TransId` are aliases for `String`. All types are re-exported at
the crate root and remain available in their respective modules.

### Markings

| Method | Behavior |
| --- | --- |
| `tokens_in(&place)` | Returns a borrowed slice, empty for an unknown place |
| `put_token(place, token)` | Inserts a token and normalizes its `place_id` |
| `remove_token(&place, id)` | Removes the first matching token, or returns `None` |
| `move_token(&from, to, id)` | Moves a token and returns its snapshot, or `None` |

Removing the last token also removes the empty bucket. A missing token leaves
the marking unchanged, and moving a token to its existing place is supported.

### Definitions

`place(id)` and `transition(id)` return the first matching definition.
`enabled_transitions(&marking)` returns borrowed transitions in definition order.

Moving a token is a marking operation: it does not look up a transition or
check whether the move is allowed by a definition. Perform domain checks in
your application before moving it.

## Serialization

All model types derive Serde's `Serialize` and `Deserialize`.
For JSON, add `serde_json` to your application:

```rust
use petri_core::{Marking, Token};

let mut marking = Marking::new();
marking.put_token("pending".into(), Token::new(1, "pending", "job-001"));

let json = serde_json::to_string(&marking).unwrap();
let restored: Marking<String> = serde_json::from_str(&json).unwrap();
assert_eq!(restored.tokens_in(&"pending".into())[0].data, "job-001");
```

## Current guarantees and limits

- Token IDs are managed by the caller; uniqueness is not enforced.
- Definitions accept duplicate IDs and dangling transition endpoints.
- Markings accept places that are absent from a definition.
- Deserialization assumes consistent input; it does not validate token bucket
  keys against each token's `place_id`.
- Source occupancy is the only enabling condition. Guards, permissions,
  capacity, deadlines, persistence and scheduling belong to the application.
- No minimum supported Rust version is claimed yet. CI uses stable Rust.

These constraints keep this first version aligned with a generic data model;
applications must validate untrusted definitions and markings before using them.

## Development

```sh
cargo fmt --check
cargo clippy --all-targets --all-features -- -D warnings
cargo test --all-targets --all-features
cargo test --doc --all-features
RUSTDOCFLAGS="-D warnings" cargo doc --no-deps --all-features
cargo run --example workflow
```

CI follows the checks used by
[rankfusion](https://github.com/Lsh0x/rankfusion), adapted to this crate:
formatting, Clippy, tests, rustdoc, dependency checks and LLVM coverage.
LCOV is saved as a workflow artifact. Codecov uploads use the optional
`CODECOV_TOKEN` secret and do not block CI if the upload fails.

Dependency auditing covers advisories, duplicate versions and allowed sources.
License auditing will be enabled once a project license is selected.
There are no benchmark targets or feature combinations to test yet.

## Releases

Release-plz is configured for release pull requests, changelog updates,
semantic-version checks, tags and GitHub releases. Both jobs remain disabled
until the repository variable `ENABLE_RELEASES` is set to `true`.

Before enabling releases:

1. Select a license and add its files and Cargo package metadata.
2. Confirm the crate name is available on crates.io.
3. Add a scoped `CARGO_REGISTRY_TOKEN` repository secret.
4. Enable **Allow GitHub Actions to create and approve pull requests** in the
   repository's Actions settings.

## License

A license has not yet been selected. Publication remains disabled.
