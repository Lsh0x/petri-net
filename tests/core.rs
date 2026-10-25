use petri_core::{Definition, Marking, Place, Token, Transition};

#[test]
fn insertion_normalizes_location_and_default_needs_no_data_default() {
    struct Data;
    let mut marking = Marking::<Data>::default();
    marking.put_token("a".into(), Token::new(7, "wrong", Data));
    assert_eq!(marking.tokens_in(&"a".into())[0].place_id, "a");
}

#[test]
fn move_preserves_payload_and_missing_move_changes_nothing() {
    let mut marking = Marking::new();
    marking.put_token("a".into(), Token::new(1, "a", "payload"));
    assert!(marking.move_token(&"a".into(), "b".into(), 99).is_none());
    assert_eq!(marking.tokens_in(&"a".into()).len(), 1);
    let moved = marking.move_token(&"a".into(), "b".into(), 1).unwrap();
    assert_eq!(moved.data, "payload");
    assert_eq!(moved.place_id, "b");
    assert!(marking.tokens_in(&"a".into()).is_empty());
    assert_eq!(marking.tokens_in(&"b".into()), &[moved]);
    marking.move_token(&"b".into(), "b".into(), 1).unwrap();
    assert_eq!(marking.tokens_in(&"b".into()).len(), 1);
    assert!(marking.remove_token(&"b".into(), 1).is_some());
    assert!(marking.tokens_in(&"b".into()).is_empty());
}

#[test]
fn enabling_is_a_pure_source_occupancy_query() {
    let net = Definition::new(
        vec![Place::new("a", ()), Place::new("b", ())],
        vec![Transition::new("ab", "a", "b", ()), Transition::new("ba", "b", "a", ())],
    );
    let mut marking = Marking::new();
    assert!(net.enabled_transitions(&marking).is_empty());
    marking.put_token("a".into(), Token::new(1, "a", ()));
    assert_eq!(net.enabled_transitions(&marking)[0].id, "ab");
    assert_eq!(marking.tokens_in(&"a".into()).len(), 1);
    assert!(net.place("missing").is_none());
    assert_eq!(net.transition("ba").unwrap().to, "a");
}

#[test]
fn generic_metadata_and_marking_round_trip() {
    let net = Definition::new(
        vec![Place::new("a", "metadata")],
        vec![Transition::new("loop", "a", "a", 42)],
    );
    let json = serde_json::to_string(&net).unwrap();
    let decoded: Definition<String, i32> = serde_json::from_str(&json).unwrap();
    assert_eq!(decoded.places[0].data, "metadata");
    let mut marking = Marking::new();
    marking.put_token("a".into(), Token::new(1, "a", vec![1, 2]));
    let json = serde_json::to_string(&marking).unwrap();
    let decoded: Marking<Vec<i32>> = serde_json::from_str(&json).unwrap();
    assert_eq!(decoded.tokens_in(&"a".into())[0].data, vec![1, 2]);
}
