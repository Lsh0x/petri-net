use petri_core::{Definition, Marking, Place, Token, Transition};
use serde_json::{json, Value};

fn id(value: &str) -> String {
    value.into()
}
fn snapshot<D: serde::Serialize>(marking: &Marking<D>) -> Value {
    serde_json::to_value(marking).unwrap()
}

#[test]
fn empty_marking_queries_and_failed_operations_are_noops() {
    let mut m = Marking::<()>::new();
    assert!(m.tokens_in(&id("missing")).is_empty());
    assert!(m.remove_token(&id("missing"), 0).is_none());
    assert!(m.move_token(&id("missing"), id("new"), 0).is_none());
    assert_eq!(snapshot(&m), json!({"tokens": {}}));
}

#[test]
fn wrong_id_in_existing_bucket_changes_nothing() {
    let mut m = Marking::new();
    m.put_token(id("a"), Token::new(1, "a", 8));
    let before = snapshot(&m);
    assert!(m.remove_token(&id("a"), 2).is_none());
    assert!(m.move_token(&id("a"), id("b"), 2).is_none());
    assert_eq!(snapshot(&m), before);
}

#[test]
fn insertion_preserves_order_and_normalizes_every_token() {
    let mut m = Marking::new();
    for n in 0..10 {
        m.put_token(id("a"), Token::new(n, "incorrect", n));
    }
    assert_eq!(
        m.tokens_in(&id("a"))
            .iter()
            .map(|t| t.id)
            .collect::<Vec<_>>(),
        (0..10).collect::<Vec<_>>()
    );
    assert!(m.tokens_in(&id("a")).iter().all(|t| t.place_id == "a"));
}

#[test]
fn removal_preserves_survivor_order_and_drops_empty_bucket() {
    let mut m = Marking::new();
    for n in 0..4 {
        m.put_token(id("a"), Token::new(n, "a", n));
    }
    for n in [1, 3, 0, 2] {
        assert_eq!(m.remove_token(&id("a"), n).unwrap().data, n);
    }
    assert_eq!(snapshot(&m), json!({"tokens": {}}));
}

#[test]
fn removing_middle_token_keeps_unrelated_places_intact() {
    let mut m = Marking::new();
    for n in 0..3 {
        m.put_token(id("a"), Token::new(n, "a", n));
    }
    m.put_token(id("b"), Token::new(99, "b", 99));
    m.remove_token(&id("a"), 1).unwrap();
    assert_eq!(
        m.tokens_in(&id("a"))
            .iter()
            .map(|t| t.id)
            .collect::<Vec<_>>(),
        vec![0, 2]
    );
    assert_eq!(m.tokens_in(&id("b"))[0].data, 99);
}

#[test]
fn duplicate_ids_remove_first_match_only() {
    let mut m = Marking::new();
    m.put_token(id("a"), Token::new(7, "a", "first"));
    m.put_token(id("a"), Token::new(7, "a", "second"));
    assert_eq!(m.remove_token(&id("a"), 7).unwrap().data, "first");
    assert_eq!(m.tokens_in(&id("a"))[0].data, "second");
}

#[test]
fn duplicate_ids_are_scoped_to_source_bucket_when_moving() {
    let mut m = Marking::new();
    m.put_token(id("a"), Token::new(7, "a", "a"));
    m.put_token(id("b"), Token::new(7, "b", "b"));
    assert_eq!(m.move_token(&id("a"), id("b"), 7).unwrap().data, "a");
    assert_eq!(
        m.tokens_in(&id("b"))
            .iter()
            .map(|t| t.data)
            .collect::<Vec<_>>(),
        vec!["b", "a"]
    );
}

#[test]
fn self_move_preserves_count_and_appends_to_bucket() {
    let mut m = Marking::new();
    for n in 0..3 {
        m.put_token(id("a"), Token::new(n, "a", n));
    }
    m.move_token(&id("a"), id("a"), 0).unwrap();
    assert_eq!(
        m.tokens_in(&id("a"))
            .iter()
            .map(|t| t.id)
            .collect::<Vec<_>>(),
        vec![1, 2, 0]
    );
}

#[test]
fn moved_snapshot_and_cloned_marking_do_not_alias_payloads() {
    let mut m = Marking::new();
    m.put_token(id("a"), Token::new(0, "a", vec![1]));
    let mut moved = m.move_token(&id("a"), id("b"), 0).unwrap();
    moved.data.push(2);
    let mut copy = m.clone();
    copy.remove_token(&id("b"), 0).unwrap();
    assert_eq!(m.tokens_in(&id("b"))[0].data, vec![1]);
}

#[test]
fn empty_unicode_and_long_place_ids_work() {
    for place in [String::new(), "收款/é/🦀\0".into(), "x".repeat(10_000)] {
        let mut m = Marking::new();
        m.put_token(place.clone(), Token::new(u64::MAX, "wrong", ()));
        assert_eq!(m.tokens_in(&place)[0].place_id, place);
        assert_eq!(m.remove_token(&place, u64::MAX).unwrap().id, u64::MAX);
    }
}

#[test]
fn non_clone_payload_can_be_inserted_queried_and_removed() {
    struct Payload(u8);
    let mut m = Marking::<Payload>::default();
    m.put_token(id("a"), Token::new(0, "a", Payload(9)));
    assert_eq!(m.tokens_in(&id("a"))[0].data.0, 9);
    assert_eq!(m.remove_token(&id("a"), 0).unwrap().data.0, 9);
}

#[test]
fn empty_definition_has_no_lookups_or_enabled_transitions() {
    let d = Definition::<(), ()>::new(vec![], vec![]);
    assert!(d.place("").is_none());
    assert!(d.transition("").is_none());
    assert!(d.enabled_transitions(&Marking::<()>::new()).is_empty());
}

#[test]
fn duplicate_definition_ids_lookup_first_entry() {
    let d = Definition::new(
        vec![Place::new("a", 1), Place::new("a", 2)],
        vec![
            Transition::new("t", "a", "b", 1),
            Transition::new("t", "b", "a", 2),
        ],
    );
    assert_eq!(d.place("a").unwrap().data, 1);
    assert_eq!(d.transition("t").unwrap().data, 1);
    assert!(d.transition("missing").is_none());
}

#[test]
fn enabling_preserves_order_includes_self_loops_and_ignores_destination() {
    let d = Definition::new(
        vec![Place::new("a", ())],
        vec![
            Transition::new("loop", "a", "a", ()),
            Transition::new("dangling", "a", "absent", ()),
            Transition::new("blocked", "b", "a", ()),
            Transition::new("other", "a", "b", ()),
        ],
    );
    let mut m = Marking::new();
    for n in 0..5 {
        m.put_token(id("a"), Token::new(n, "a", ()));
    }
    let before = snapshot(&m);
    assert_eq!(
        d.enabled_transitions(&m)
            .iter()
            .map(|t| t.id.as_str())
            .collect::<Vec<_>>(),
        vec!["loop", "dangling", "other"]
    );
    assert_eq!(snapshot(&m), before);
}

#[test]
fn marking_and_definition_allow_unknown_places_without_validation() {
    let d = Definition::<(), ()>::new(
        vec![],
        vec![Transition::new("t", "unknown", "elsewhere", ())],
    );
    let mut m = Marking::new();
    m.put_token(id("unknown"), Token::new(1, "unknown", ()));
    assert_eq!(d.enabled_transitions(&m).len(), 1);
    assert!(m
        .move_token(&id("unknown"), id("not-in-definition"), 1)
        .is_some());
}

#[test]
fn serialization_supports_extreme_ids_and_nested_payload() {
    let mut m = Marking::new();
    for n in [0, u64::MAX] {
        m.put_token(
            id("a"),
            Token::new(n, "a", json!({"nested": [null, true, "🦀"]})),
        );
    }
    let restored: Marking<Value> = serde_json::from_value(snapshot(&m)).unwrap();
    assert_eq!(m.tokens_in(&id("a")), restored.tokens_in(&id("a")));
}

#[test]
fn deserialized_inconsistent_bucket_is_documented_not_silently_validated() {
    let mut m: Marking<()> = serde_json::from_value(
        json!({"tokens": {"bucket": [{"id": 1, "place_id": "wrong", "data": null}]}}),
    )
    .unwrap();
    assert_eq!(m.tokens_in(&id("bucket"))[0].place_id, "wrong");
    m.move_token(&id("bucket"), id("fixed"), 1).unwrap();
    assert_eq!(m.tokens_in(&id("fixed"))[0].place_id, "fixed");
}

#[test]
fn malformed_marking_json_is_rejected() {
    for value in [
        json!({}),
        json!({"tokens": []}),
        json!({"tokens": {"a": [{}]}}),
        json!({"tokens": {"a": [{"id": -1, "place_id": "a", "data": null}]}}),
        json!({"tokens": {"a": [{"id": "one", "place_id": "a", "data": null}]}}),
    ] {
        assert!(serde_json::from_value::<Marking<()>>(value).is_err());
    }
}

#[test]
fn constructors_and_serde_preserve_model_fields() {
    let place = Place::new("🦀", vec![1, 2]);
    let transition = Transition::new("", "🦀", "end", 42);
    let token = Token::new(u64::MAX, "🦀", "data".to_string());
    assert_eq!(
        place,
        serde_json::from_value::<Place<Vec<i32>>>(serde_json::to_value(&place).unwrap()).unwrap()
    );
    assert_eq!(
        transition,
        serde_json::from_value::<Transition<i32>>(serde_json::to_value(&transition).unwrap())
            .unwrap()
    );
    assert_eq!(
        token,
        serde_json::from_value::<Token<String>>(serde_json::to_value(&token).unwrap()).unwrap()
    );
}

#[test]
fn ten_thousand_tokens_survive_repeated_moves() {
    let mut m = Marking::new();
    for n in 0..10_000 {
        m.put_token(id("a"), Token::new(n, "a", n));
    }
    for n in 0..10_000 {
        assert_eq!(m.move_token(&id("a"), id("b"), n).unwrap().data, n);
    }
    assert!(m.tokens_in(&id("a")).is_empty());
    assert_eq!(m.tokens_in(&id("b")).len(), 10_000);
}
